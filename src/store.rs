//! Turso (embedded, SQLite-compatible) storage for thesaurus senses.

use crate::model::{Entry, Pos};
use anyhow::{Context, Result};
use std::path::Path;
use tokio::runtime::Runtime;
use turso::{Builder, Connection, Database};

pub const SCHEMA: &str = "
CREATE TABLE meta (k TEXT PRIMARY KEY, v TEXT NOT NULL);
CREATE TABLE senses (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL,
    word TEXT NOT NULL,
    pos TEXT NOT NULL,
    wordnet_id TEXT NOT NULL,
    gloss TEXT NOT NULL,
    examples TEXT NOT NULL,
    lemmas TEXT NOT NULL
);
CREATE TABLE members (lemma TEXT NOT NULL, sid INTEGER NOT NULL);
";

const INDEXES: &str = "
CREATE UNIQUE INDEX senses_key ON senses(key);
CREATE INDEX members_lemma ON members(lemma, sid);
";

const SENSE_COLS: &str = "s.id, s.key, s.word, s.pos, s.gloss, s.examples, s.lemmas";

/// A sense as stored, with every member of its synset.
#[derive(Debug, Clone)]
pub struct SenseRow {
    pub id: i64,
    pub key: String,
    pub word: String,
    pub pos: Pos,
    pub gloss: String,
    pub examples: Vec<String>,
    /// Display forms of all synset members, headword first.
    pub lemmas: Vec<String>,
}

pub struct Store {
    rt: Runtime,
    _db: Database,
    conn: Connection,
}

fn runtime() -> Result<Runtime> {
    Ok(tokio::runtime::Builder::new_current_thread().build()?)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let rt = runtime()?;
        let path_str = path.to_str().context("database path is not valid UTF-8")?;
        let db = rt
            .block_on(Builder::new_local(path_str).read_only(true).build())
            .with_context(|| format!("opening {}", path.display()))?;
        let conn = db.connect()?;
        Ok(Store { rt, _db: db, conn })
    }

    pub fn meta(&self, k: &str) -> Result<Option<String>> {
        self.rt.block_on(async {
            let mut stmt = self.conn.prepare_cached("SELECT v FROM meta WHERE k = ?1").await?;
            let mut rows = stmt.query([k]).await?;
            Ok(match rows.next().await? {
                Some(row) => Some(row.get::<String>(0)?),
                None => None,
            })
        })
    }

    /// Every sense whose synset contains `lemma` (lowercase, underscored).
    pub fn senses_for_lemma(&self, lemma: &str) -> Result<Vec<SenseRow>> {
        let sql = format!(
            "SELECT {SENSE_COLS} FROM members m JOIN senses s ON s.id = m.sid \
             WHERE m.lemma = ?1 ORDER BY lower(s.word) != ?1, s.id"
        );
        self.rows(&sql, lemma)
    }

    pub fn sense_by_key(&self, key: &str) -> Result<Option<SenseRow>> {
        let sql = format!("SELECT {SENSE_COLS} FROM senses s WHERE s.key = ?1");
        Ok(self.rows(&sql, key)?.into_iter().next())
    }

    fn rows(&self, sql: &str, arg: &str) -> Result<Vec<SenseRow>> {
        self.rt.block_on(async {
            let mut stmt = self.conn.prepare_cached(sql).await?;
            let mut rows = stmt.query([arg]).await?;
            let mut out = Vec::new();
            while let Some(row) = rows.next().await? {
                let pos: String = row.get(3)?;
                let examples: String = row.get(5)?;
                let lemmas: String = row.get(6)?;
                out.push(SenseRow {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    word: row.get(2)?,
                    pos: pos.parse().map_err(anyhow::Error::msg)?,
                    gloss: row.get(4)?,
                    examples: examples
                        .split('\n')
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                    lemmas: lemmas.split(' ').map(str::to_string).collect(),
                });
            }
            Ok(out)
        })
    }
}

/// Write a fresh database at `path` from sorted entries.
///
/// `forms(entry)` yields the display form of each synset member, headword first.
pub fn write<'a, F>(path: &Path, entries: &'a [Entry], forms: F, meta: &[(&str, String)]) -> Result<()>
where
    F: Fn(&'a Entry) -> Vec<String>,
{
    let rt = runtime()?;
    let path_str = path.to_str().context("database path is not valid UTF-8")?;
    rt.block_on(async {
        let db = Builder::new_local(path_str).build().await?;
        let conn = db.connect()?;
        conn.execute_batch(SCHEMA).await?;
        conn.execute("BEGIN", ()).await?;
        {
            let mut meta_stmt = conn.prepare("INSERT INTO meta (k, v) VALUES (?1, ?2)").await?;
            for (k, v) in meta {
                meta_stmt.execute([*k, v.as_str()]).await?;
            }
            let mut sense = conn
                .prepare(
                    "INSERT INTO senses (id, key, word, pos, wordnet_id, gloss, examples, lemmas) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                )
                .await?;
            let mut member = conn
                .prepare("INSERT INTO members (lemma, sid) VALUES (?1, ?2)")
                .await?;
            for (i, e) in entries.iter().enumerate() {
                let id = i as i64 + 1;
                let examples = e
                    .examples
                    .iter()
                    .map(|x| x.replace('\n', " "))
                    .collect::<Vec<_>>()
                    .join("\n");
                let lemmas = forms(e).join(" ");
                sense
                    .execute((
                        id,
                        e.key.as_str(),
                        e.word.as_str(),
                        e.pos.as_str(),
                        e.wordnet_id.as_str(),
                        e.gloss.as_str(),
                        examples,
                        lemmas,
                    ))
                    .await?;
                for lemma in e.lemmas() {
                    member.execute((lemma, id)).await?;
                }
            }
        }
        conn.execute("COMMIT", ()).await?;
        conn.execute_batch(INDEXES).await?;
        // Fold the WAL into the main file so read-only opens are clean.
        let _ = conn.execute("PRAGMA wal_checkpoint(TRUNCATE)", ()).await;
        anyhow::Ok(())
    })
}
