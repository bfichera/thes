//! Build the Turso database and Tantivy index from the source `jsonl`.

use crate::model::{Entry, Pos, RawEntry};
use crate::paths::{DEFAULT_SOURCE, Paths, SCHEMA_VERSION};
use crate::more;
use crate::search::{self, LemmaDoc};
use crate::store;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
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

/// Where thesaurus data comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// The WordNet jsonl compiled into the binary.
    Embedded,
    /// A local `.jsonl` or zstd-compressed `.jsonl.zst` file.
    File(PathBuf),
}

impl Source {
    pub fn describe(&self) -> String {
        match self {
            Source::Embedded => "embedded WordNet 3.0 (zaibacu/thesaurus)".into(),
            Source::File(p) => p.display().to_string(),
        }
    }

    fn reader(&self) -> Result<Box<dyn Read>> {
        match self {
            #[cfg(feature = "embed-data")]
            Source::Embedded => Ok(Box::new(zstd_reader(EMBEDDED)?)),
            #[cfg(not(feature = "embed-data"))]
            Source::Embedded => bail!("this build has no embedded data"),
            Source::File(p) => {
                let f = std::fs::File::open(p).with_context(|| format!("opening {}", p.display()))?;
                if p.extension().is_some_and(|e| e == "zst") {
                    Ok(Box::new(zstd_reader(BufReader::new(f))?))
                } else {
                    Ok(Box::new(f))
                }
            }
        }
    }
}

#[cfg(feature = "embed-data")]
const EMBEDDED: &[u8] = include_bytes!("../data/en_thesaurus.jsonl.zst");

#[cfg(feature = "embed-data")]
const EMBEDDED_MOBY: &[u8] = include_bytes!("../data/moby.txt.zst");

#[cfg(feature = "embed-data")]
const EMBEDDED_RELATIONS: &[u8] = include_bytes!("../data/en_relations.tsv.zst");

/// Most related words kept per sense.
const MAX_RELATED: usize = 40;

/// Moby Thesaurus text. Only the embedded source ships it.
fn read_moby(source: &Source) -> Result<Option<String>> {
    #[cfg(feature = "embed-data")]
    if matches!(source, Source::Embedded) {
        let mut text = String::new();
        zstd_reader(EMBEDDED_MOBY)?.read_to_string(&mut text)?;
        return Ok(Some(text));
    }
    let _ = source;
    Ok(None)
}

/// Synset relations as `(from_key, kind, to_key)`. Only the embedded source ships them.
fn read_relations(source: &Source) -> Result<Vec<(String, String, String)>> {
    #[cfg(feature = "embed-data")]
    if matches!(source, Source::Embedded) {
        let mut text = String::new();
        zstd_reader(EMBEDDED_RELATIONS)?.read_to_string(&mut text)?;
        return Ok(text
            .lines()
            .filter_map(|l| {
                let mut it = l.split('\t');
                Some((it.next()?.into(), it.next()?.into(), it.next()?.into()))
            })
            .collect());
    }
    let _ = source;
    Ok(Vec::new())
}

fn zstd_reader<R: Read>(r: R) -> Result<impl Read> {
    ruzstd::decoding::StreamingDecoder::new(r)
        .map_err(|e| anyhow::anyhow!("invalid zstd data: {e}"))
}

pub const HAS_EMBEDDED: bool = cfg!(feature = "embed-data");

/// Pick a data source. Order: explicit `source`, `$THES_SOURCE`, embedded data,
/// a previously downloaded copy, then a fresh download of the default.
pub fn resolve_source(paths: &Paths, source: Option<&str>, log: &dyn Fn(&str)) -> Result<Source> {
    let env = std::env::var("THES_SOURCE").ok().filter(|s| !s.is_empty());
    match source.map(str::to_string).or(env) {
        Some(s) if s == "embedded" => Ok(Source::Embedded),
        Some(s) if is_url(&s) => Ok(Source::File(download(&s, &paths.source_cache(), log)?)),
        Some(s) => {
            let p = PathBuf::from(&s);
            if !p.is_file() {
                bail!("source file not found: {}", p.display());
            }
            Ok(Source::File(p))
        }
        None if HAS_EMBEDDED => Ok(Source::Embedded),
        None => {
            let cache = paths.source_cache();
            if !cache.is_file() {
                download(DEFAULT_SOURCE, &cache, log)?;
            }
            Ok(Source::File(cache))
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

pub fn read_entries(source: &Source) -> Result<(Vec<Entry>, usize)> {
    let name = source.describe();
    let mut entries = Vec::with_capacity(120_000);
    let mut seen_keys = BTreeSet::new();
    let mut skipped = 0;
    for (i, line) in BufReader::new(source.reader()?).lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let raw: RawEntry = serde_json::from_str(&line)
            .with_context(|| format!("{name}:{}: invalid JSON", i + 1))?;
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
pub fn build(paths: &Paths, source: &Source, log: &dyn Fn(&str)) -> Result<Report> {
    let start = Instant::now();
    log(&format!("reading {}", source.describe()));
    let (entries, skipped) = read_entries(source)?;
    if entries.is_empty() {
        bail!("no entries found in {}", source.describe());
    }

    // Headwords keep their capitalisation (e.g. "Paris"); synonyms are lowercase in the
    // source, so borrow the headword's casing for display wherever it exists. A
    // lowercase headword wins over "BASIC" / "Capital" style ones, and WordNet's
    // `(a)` markers are dropped.
    let mut display: HashMap<String, &str> = HashMap::new();
    for e in &entries {
        let name = more::strip_marker(&e.word);
        let slot = display.entry(name.to_lowercase()).or_insert(name);
        if *slot != name && name.chars().all(|c| !c.is_uppercase()) {
            *slot = name;
        }
    }
    let form = |lemma: &str, orig: &'_ str| -> String {
        let c = more::clean(lemma);
        display.get(&c).map(|s| s.to_string()).unwrap_or_else(|| more::strip_marker(orig).to_string())
    };

    // Words from similar / see-also / broader synsets, nearest relation first.
    let index: HashMap<&str, usize> =
        entries.iter().enumerate().map(|(i, e)| (e.key.as_str(), i)).collect();
    let mut linked: Vec<[Vec<usize>; 3]> = vec![Default::default(); entries.len()];
    for (from, kind, to) in read_relations(source)? {
        let slot = match kind.as_str() {
            "sim" => 0,
            "see" => 1,
            "hyper" => 2,
            _ => continue,
        };
        if let (Some(&a), Some(&b)) = (index.get(from.as_str()), index.get(to.as_str())) {
            linked[a][slot].push(b);
            if slot < 2 {
                linked[b][slot].push(a);
            }
        }
    }
    let nbrs: Vec<Vec<usize>> = linked.iter().map(|g| g.iter().flatten().copied().collect()).collect();
    let related: Vec<Vec<String>> = entries
        .iter()
        .zip(&linked)
        .map(|(e, groups)| {
            let mut seen: BTreeSet<String> = e.lemmas().map(|l| more::clean(&l)).collect();
            let mut out = Vec::new();
            for &j in groups.iter().flatten() {
                let t = &entries[j];
                for (lemma, orig) in t.lemmas().zip(std::iter::once(&t.word).chain(&t.synonyms)) {
                    if out.len() < MAX_RELATED && seen.insert(more::clean(&lemma)) {
                        out.push(form(&lemma, orig));
                    }
                }
            }
            out
        })
        .collect();

    let more: Vec<Vec<String>> = match read_moby(source)? {
        Some(text) => {
            let moby = more::Moby::parse(&text);
            let shown: Vec<HashSet<String>> = entries
                .iter()
                .zip(&related)
                .map(|(e, rel)| {
                    e.lemmas()
                        .map(|l| more::clean(&l))
                        .chain(rel.iter().map(|r| more::clean(r)))
                        .collect()
                })
                .collect();
            more::compute(&entries, &nbrs, &shown, &moby)
                .into_iter()
                .map(|v| v.into_iter().map(|w| form(&w, &w)).collect())
                .collect()
        }
        None => vec![Vec::new(); entries.len()],
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
        ("source", source.describe()),
        ("thes_version", env!("CARGO_PKG_VERSION").to_string()),
        ("senses", entries.len().to_string()),
    ];
    store::write(
        &tmp_paths.db(),
        &entries,
        |e| {
            let mut forms = vec![form(&e.word.to_lowercase(), &e.word)];
            let mut seen = HashSet::from([more::clean(&e.word)]);
            for s in &e.synonyms {
                if seen.insert(more::clean(s)) {
                    forms.push(form(&s.to_lowercase(), s));
                }
            }
            forms
        },
        &related,
        &more,
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
    std::fs::write(tmp_paths.stamp(), crate::paths::stamp())?;

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
