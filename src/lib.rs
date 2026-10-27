//! `thes`: a fast offline thesaurus.
//!
//! ```no_run
//! use thes::{Thesaurus, Query};
//! let t = Thesaurus::open_default()?;
//! for hit in t.search(&Query::parse("hapy pos:adj").unwrap(), 10)? {
//!     println!("{} {:?}", hit.word, hit.pos);
//! }
//! let page = t.page("happy", &Default::default())?;
//! # anyhow::Ok(())
//! ```

pub mod ingest;
pub mod model;
pub mod paths;
pub mod search;
pub mod store;

pub use model::{Filter, Hit, Page, Pos, Query, Sense, display, normalize};
pub use paths::Paths;

use anyhow::{Result, bail};
use search::SearchIndex;
use store::{SenseRow, Store};

pub struct Thesaurus {
    store: Store,
    index: SearchIndex,
}

impl Thesaurus {
    pub fn open(paths: &Paths) -> Result<Thesaurus> {
        if !paths.is_current() {
            bail!(
                "no thesaurus data at {} (run `thes build`)",
                paths.root.display()
            );
        }
        Ok(Thesaurus {
            store: Store::open(&paths.db())?,
            index: SearchIndex::open(&paths.index())?,
        })
    }

    pub fn open_default() -> Result<Thesaurus> {
        Thesaurus::open(&Paths::from_env()?)
    }

    /// Open, building the data first (downloading the source if needed) when missing.
    pub fn open_or_build(paths: &Paths, log: &dyn Fn(&str)) -> Result<Thesaurus> {
        if !paths.is_current() {
            let src = ingest::fetch_source(paths, None, log)?;
            let r = ingest::build(paths, &src, log)?;
            log(&format!(
                "built {} senses / {} words in {:.1}s",
                r.senses,
                r.lemmas,
                r.millis as f64 / 1000.0
            ));
        }
        Thesaurus::open(paths)
    }

    /// Fuzzy word search. `query.text` may be empty when filtering by key or part of speech.
    pub fn search(&self, query: &Query, limit: usize) -> Result<Vec<Hit>> {
        self.index.search(&query.text, &query.filter, limit)
    }

    /// The thesaurus page for an exact word: every sense it belongs to, with its synonyms.
    pub fn page(&self, word: &str, filter: &Filter) -> Result<Page> {
        let lemma = normalize(word);
        let rows = self.store.senses_for_lemma(&lemma)?;
        let mut shown = None;
        let mut senses: Vec<Sense> = rows
            .into_iter()
            .filter(|r| filter.pos.is_none_or(|p| p == r.pos))
            .filter(|r| filter.key.as_deref().is_none_or(|k| k == r.key))
            .map(|r| {
                if shown.is_none() {
                    shown = r.lemmas.iter().find(|l| l.to_lowercase() == lemma).cloned();
                }
                to_sense(r, &lemma)
            })
            .collect();
        // Group by part of speech, in order of first appearance, keeping rank within each.
        let mut order: Vec<Pos> = Vec::new();
        for s in &senses {
            if !order.contains(&s.pos) {
                order.push(s.pos);
            }
        }
        senses.sort_by_key(|s| order.iter().position(|p| *p == s.pos));
        Ok(Page {
            word: shown.unwrap_or(lemma),
            senses,
        })
    }

    /// A single sense by its key, e.g. `happy_2`.
    pub fn sense(&self, key: &str) -> Result<Option<Sense>> {
        Ok(self.store.sense_by_key(key)?.map(|r| {
            let head = r.word.to_lowercase();
            to_sense(r, &head)
        }))
    }

    pub fn meta(&self, k: &str) -> Result<Option<String>> {
        self.store.meta(k)
    }
}

fn to_sense(r: SenseRow, lemma: &str) -> Sense {
    Sense {
        synonyms: r
            .lemmas
            .into_iter()
            .filter(|l| l.to_lowercase() != lemma)
            .collect(),
        key: r.key,
        pos: r.pos,
        headword: r.word,
        gloss: r.gloss,
        examples: r.examples,
    }
}
