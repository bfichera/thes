//! Tantivy fuzzy index over every lemma (headwords and synonyms).

use crate::model::{Filter, Hit, Pos, normalize};
use anyhow::{Context, Result};
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::{
    AllQuery, BooleanQuery, ConstScoreQuery, FuzzyTermQuery, Occur, Query, TermQuery,
};
use tantivy::schema::{
    FAST, Field, IndexRecordOption, STORED, STRING, Schema, TEXT, TantivyDocument, Value,
};
use tantivy::{DocId, Index, IndexReader, IndexWriter, ReloadPolicy, Score, SegmentReader, Term};

#[derive(Clone, Copy)]
struct Fields {
    /// Whole lowercase lemma, e.g. `ice_cream`.
    word: Field,
    display: Field,
    /// Lemma split into tokens, so `cream` finds `ice_cream`.
    parts: Field,
    pos: Field,
    key: Field,
    senses: Field,
    len: Field,
    proper: Field,
    gloss: Field,
}

fn schema() -> (Schema, Fields) {
    let mut b = Schema::builder();
    let f = Fields {
        word: b.add_text_field("word", STRING | STORED),
        display: b.add_text_field("display", STORED),
        parts: b.add_text_field("parts", TEXT),
        pos: b.add_text_field("pos", STRING | STORED),
        key: b.add_text_field("key", STRING),
        senses: b.add_u64_field("senses", FAST | STORED),
        len: b.add_u64_field("len", FAST),
        proper: b.add_u64_field("proper", FAST),
        gloss: b.add_text_field("gloss", STORED),
    };
    (b.build(), f)
}

fn fields(schema: &Schema) -> Result<Fields> {
    let g = |n: &str| schema.get_field(n).with_context(|| format!("index missing field '{n}'"));
    Ok(Fields {
        word: g("word")?,
        display: g("display")?,
        parts: g("parts")?,
        pos: g("pos")?,
        key: g("key")?,
        senses: g("senses")?,
        len: g("len")?,
        proper: g("proper")?,
        gloss: g("gloss")?,
    })
}

/// One searchable lemma.
pub struct LemmaDoc {
    pub lemma: String,
    pub display: String,
    pub pos: Vec<Pos>,
    pub keys: Vec<String>,
    pub senses: u64,
    pub gloss: String,
}

pub fn write(dir: &Path, docs: impl IntoIterator<Item = LemmaDoc>) -> Result<usize> {
    std::fs::create_dir_all(dir)?;
    let (schema, f) = schema();
    let index = Index::create_in_dir(dir, schema)?;
    let mut w: IndexWriter = index.writer_with_num_threads(2, 128_000_000)?;
    let mut n = 0;
    for d in docs {
        let mut doc = TantivyDocument::default();
        doc.add_text(f.word, &d.lemma);
        doc.add_text(f.display, &d.display);
        doc.add_text(f.parts, &d.lemma);
        for p in &d.pos {
            doc.add_text(f.pos, p.as_str());
        }
        for k in &d.keys {
            doc.add_text(f.key, k);
        }
        doc.add_u64(f.senses, d.senses);
        doc.add_u64(f.len, d.lemma.chars().count() as u64);
        doc.add_u64(f.proper, d.display.chars().any(char::is_uppercase) as u64);
        doc.add_text(f.gloss, &d.gloss);
        w.add_document(doc)?;
        n += 1;
    }
    w.commit()?;
    w.wait_merging_threads()?;
    Ok(n)
}

pub struct SearchIndex {
    reader: IndexReader,
    f: Fields,
}

fn scored(q: impl Query + 'static, score: Score) -> Box<dyn Query> {
    Box::new(ConstScoreQuery::new(Box::new(q), score))
}

impl SearchIndex {
    pub fn open(dir: &Path) -> Result<SearchIndex> {
        let index = Index::open_in_dir(dir).with_context(|| format!("opening {}", dir.display()))?;
        let f = fields(&index.schema())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(SearchIndex { reader, f })
    }

    /// Fuzzy, prefix and token search, ranked roughly as:
    /// exact > prefix > 1 typo > 2 typos > partial word, then by sense count.
    pub fn search(&self, text: &str, filter: &Filter, limit: usize) -> Result<Vec<Hit>> {
        let f = self.f;
        let q = normalize(text);
        let qlen = q.chars().count();
        let mut should: Vec<Box<dyn Query>> = Vec::new();
        if !q.is_empty() {
            let t = Term::from_field_text(f.word, &q);
            should.push(scored(TermQuery::new(t.clone(), IndexRecordOption::Basic), 100.0));
            should.push(scored(FuzzyTermQuery::new_prefix(t.clone(), 0, true), 20.0));
            if qlen >= 3 {
                should.push(scored(FuzzyTermQuery::new(t.clone(), 1, true), 10.0));
            }
            if qlen >= 6 {
                should.push(scored(FuzzyTermQuery::new(t.clone(), 2, true), 4.0));
            }
            if qlen >= 5 {
                should.push(scored(FuzzyTermQuery::new_prefix(t, 1, true), 3.0));
            }
            for tok in q.split(|c: char| !c.is_alphanumeric()).filter(|s| !s.is_empty()) {
                let t = Term::from_field_text(f.parts, tok);
                should.push(scored(TermQuery::new(t.clone(), IndexRecordOption::Basic), 2.0));
                if tok.chars().count() >= 4 {
                    should.push(scored(FuzzyTermQuery::new(t, 1, true), 1.0));
                }
            }
        }

        let text_query: Box<dyn Query> = if should.is_empty() {
            Box::new(AllQuery)
        } else {
            Box::new(BooleanQuery::new(
                should.into_iter().map(|q| (Occur::Should, q)).collect(),
            ))
        };
        let mut clauses = vec![(Occur::Must, text_query)];
        if let Some(p) = filter.pos {
            let t = Term::from_field_text(f.pos, p.as_str());
            clauses.push((Occur::Must, scored(TermQuery::new(t, IndexRecordOption::Basic), 0.0)));
        }
        if let Some(k) = &filter.key {
            let t = Term::from_field_text(f.key, k);
            clauses.push((Occur::Must, scored(TermQuery::new(t, IndexRecordOption::Basic), 0.0)));
        }
        let query = BooleanQuery::new(clauses);

        let collector = TopDocs::with_limit(limit.max(1)).tweak_score(move |seg: &SegmentReader| {
            let ff = seg.fast_fields();
            let senses = ff.u64("senses").expect("senses fast field").first_or_default_col(0);
            let len = ff.u64("len").expect("len fast field").first_or_default_col(0);
            let proper = ff.u64("proper").expect("proper fast field").first_or_default_col(0);
            move |doc: DocId, score: Score| {
                let s = senses.get_val(doc) as f32;
                let d = (len.get_val(doc) as f32 - qlen as f32).abs();
                let p = proper.get_val(doc) as f32;
                score + (1.0 + s).ln() - 0.15 * d - 1.5 * p
            }
        });

        let searcher = self.reader.searcher();
        let top = searcher.search(&query, &collector)?;
        let mut hits = Vec::with_capacity(top.len());
        for (score, addr) in top {
            let doc: TantivyDocument = searcher.doc(addr)?;
            let text = |field| {
                doc.get_first(field)
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_default()
            };
            hits.push(Hit {
                word: text(f.display),
                pos: doc
                    .get_all(f.pos)
                    .filter_map(|v| v.as_str().and_then(|s| s.parse().ok()))
                    .collect(),
                senses: doc.get_first(f.senses).and_then(|v| v.as_u64()).unwrap_or(0),
                gloss: text(f.gloss),
                score,
            });
        }
        Ok(hits)
    }
}
