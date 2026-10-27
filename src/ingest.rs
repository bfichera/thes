//! Build the Turso database and Tantivy index from the source `jsonl`.

use crate::model::{Entry, Pos, RawEntry};
use crate::paths::{DEFAULT_SOURCE, Paths, SCHEMA_VERSION};
use crate::search::{self, LemmaDoc};
use crate::store;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

#[derive(Debug, Default)]
pub struct Report {
    pub senses: usize,
    pub lemmas: usize,
    pub skipped: usize,
    pub millis: u128,
}

/// Resolve a source (local path or http(s) URL) to a local file, downloading if needed.
pub fn fetch_source(paths: &Paths, source: Option<&str>, log: &dyn Fn(&str)) -> Result<PathBuf> {
    let env = std::env::var("THES_SOURCE").ok().filter(|s| !s.is_empty());
    let src = source.map(str::to_string).or(env);
    match src {
        Some(s) if !is_url(&s) => {
            let p = PathBuf::from(&s);
            if !p.is_file() {
                bail!("source file not found: {}", p.display());
            }
            Ok(p)
        }
        Some(url) => download(&url, &paths.source_cache(), log),
        None => {
            let cache = paths.source_cache();
            if cache.is_file() {
                Ok(cache)
            } else {
                download(DEFAULT_SOURCE, &cache, log)
            }
        }
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn download(url: &str, dest: &Path, log: &dyn Fn(&str)) -> Result<PathBuf> {
    std::fs::create_dir_all(dest.parent().unwrap_or(Path::new(".")))?;
    log(&format!("downloading {url}"));
    let tmp = dest.with_extension("part");
    let status = Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&tmp)
        .arg(url)
        .status()
        .context("running curl (is it installed?)")?;
    if !status.success() {
        let _ = std::fs::remove_file(&tmp);
        bail!("download failed: {url}");
    }
    std::fs::rename(&tmp, dest)?;
    Ok(dest.to_path_buf())
}

fn sense_number(key: &str) -> u32 {
    key.rsplit_once('_')
        .and_then(|(_, n)| n.parse().ok())
        .unwrap_or(u32::MAX)
}

pub fn read_entries(path: &Path) -> Result<(Vec<Entry>, usize)> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut entries = Vec::with_capacity(120_000);
    let mut seen_keys = BTreeSet::new();
    let mut skipped = 0;
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw: RawEntry = serde_json::from_str(&line)
            .with_context(|| format!("{}:{}: invalid JSON", path.display(), i + 1))?;
        match Entry::from_raw(raw) {
            Some(e) if seen_keys.insert(e.key.clone()) => entries.push(e),
            _ => skipped += 1,
        }
    }
    entries.sort_by(|a, b| {
        (a.word.to_lowercase(), sense_number(&a.key), &a.key)
            .cmp(&(b.word.to_lowercase(), sense_number(&b.key), &b.key))
    });
    Ok((entries, skipped))
}

/// Build into a temporary directory, then swap it in atomically.
pub fn build(paths: &Paths, source: &Path, log: &dyn Fn(&str)) -> Result<Report> {
    let start = Instant::now();
    log(&format!("reading {}", source.display()));
    let (entries, skipped) = read_entries(source)?;
    if entries.is_empty() {
        bail!("no entries found in {}", source.display());
    }

    // Headwords keep their capitalisation (e.g. "Paris"); synonyms are lowercase in the
    // source, so borrow the headword's casing for display wherever it exists.
    let mut display: HashMap<String, &str> = HashMap::new();
    for e in &entries {
        display.entry(e.word.to_lowercase()).or_insert(&e.word);
    }
    let form = |lemma: &str, orig: &'_ str| -> String {
        display.get(lemma).map(|s| s.to_string()).unwrap_or_else(|| orig.to_string())
    };

    let parent = paths.root.clone();
    std::fs::create_dir_all(&parent)?;
    let tmp = parent.join(".build");
    if tmp.exists() {
        std::fs::remove_dir_all(&tmp)?;
    }
    std::fs::create_dir_all(&tmp)?;
    let tmp_paths = Paths::new(&tmp);

    log(&format!("writing {} senses to turso database", entries.len()));
    let meta = [
        ("schema_version", SCHEMA_VERSION.to_string()),
        ("source", source.display().to_string()),
        ("senses", entries.len().to_string()),
    ];
    store::write(
        &tmp_paths.db(),
        &entries,
        |e| {
            std::iter::once(&e.word)
                .chain(e.synonyms.iter())
                .map(|s| form(&s.to_lowercase(), s))
                .collect()
        },
        &meta,
    )?;

    struct Acc {
        display: String,
        pos: BTreeSet<Pos>,
        keys: Vec<String>,
        gloss: String,
        own_gloss: bool,
    }
    let mut lemmas: BTreeMap<String, Acc> = BTreeMap::new();
    for e in &entries {
        for (lemma, orig) in e.lemmas().zip(std::iter::once(&e.word).chain(e.synonyms.iter())) {
            let acc = lemmas.entry(lemma.clone()).or_insert_with(|| Acc {
                display: form(&lemma, orig),
                pos: BTreeSet::new(),
                keys: Vec::new(),
                gloss: String::new(),
                own_gloss: false,
            });
            acc.pos.insert(e.pos);
            acc.keys.push(e.key.clone());
            let own = lemma == e.word.to_lowercase();
            if acc.gloss.is_empty() || (own && !acc.own_gloss) {
                acc.gloss = e.gloss.clone();
                acc.own_gloss = own;
            }
        }
    }

    log(&format!("indexing {} words with tantivy", lemmas.len()));
    let n = search::write(
        &tmp_paths.index(),
        lemmas.into_iter().map(|(lemma, a)| LemmaDoc {
            senses: a.keys.len() as u64,
            lemma,
            display: a.display,
            pos: a.pos.into_iter().collect(),
            keys: a.keys,
            gloss: a.gloss,
        }),
    )?;
    std::fs::write(tmp_paths.stamp(), SCHEMA_VERSION)?;

    for p in [paths.db(), paths.index(), paths.stamp()] {
        if p.is_dir() {
            std::fs::remove_dir_all(&p)?;
        } else if p.exists() {
            std::fs::remove_file(&p)?;
        }
    }
    for suffix in ["-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", paths.db().display()));
    }
    std::fs::rename(tmp_paths.db(), paths.db())?;
    std::fs::rename(tmp_paths.index(), paths.index())?;
    std::fs::rename(tmp_paths.stamp(), paths.stamp())?;
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(Report {
        senses: entries.len(),
        lemmas: n,
        skipped,
        millis: start.elapsed().as_millis(),
    })
}
