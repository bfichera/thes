use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pos {
    Noun,
    Verb,
    Adj,
    Adv,
}

impl Pos {
    pub const ALL: [Pos; 4] = [Pos::Noun, Pos::Verb, Pos::Adj, Pos::Adv];

    pub fn as_str(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Verb => "verb",
            Pos::Adj => "adj",
            Pos::Adv => "adv",
        }
    }

    pub fn long(self) -> &'static str {
        match self {
            Pos::Noun => "noun",
            Pos::Verb => "verb",
            Pos::Adj => "adjective",
            Pos::Adv => "adverb",
        }
    }
}

impl fmt::Display for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Pos {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "n" | "noun" => Ok(Pos::Noun),
            "v" | "verb" => Ok(Pos::Verb),
            "a" | "s" | "adj" | "adjective" => Ok(Pos::Adj),
            "r" | "adv" | "adverb" => Ok(Pos::Adv),
            other => Err(format!(
                "unknown part of speech '{other}' (expected noun, verb, adj, adv)"
            )),
        }
    }
}

/// One line of the source `jsonl` file.
#[derive(Debug, Clone, Deserialize)]
pub struct RawEntry {
    pub word: String,
    pub wordnet_id: String,
    pub key: String,
    pub pos: String,
    #[serde(default)]
    pub synonyms: Vec<String>,
    #[serde(default)]
    pub desc: Vec<String>,
}

/// A cleaned sense ready to be stored.
#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub word: String,
    pub pos: Pos,
    pub wordnet_id: String,
    pub gloss: String,
    pub examples: Vec<String>,
    /// Other members of the synset, excluding the headword itself.
    pub synonyms: Vec<String>,
}

impl Entry {
    pub fn from_raw(raw: RawEntry) -> Option<Entry> {
        let pos = raw.pos.parse().ok()?;
        let head_lc = raw.word.to_lowercase();
        let mut seen = vec![head_lc];
        let mut synonyms = Vec::new();
        for s in raw.synonyms {
            let lc = s.to_lowercase();
            if !seen.contains(&lc) {
                seen.push(lc);
                synonyms.push(s);
            }
        }
        let (gloss, examples) = split_desc(&raw.desc);
        Some(Entry {
            key: raw.key,
            word: raw.word,
            pos,
            wordnet_id: raw.wordnet_id,
            gloss,
            examples,
            synonyms,
        })
    }

    /// All lowercase lemmas belonging to this synset (headword first).
    pub fn lemmas(&self) -> impl Iterator<Item = String> + '_ {
        std::iter::once(&self.word)
            .chain(self.synonyms.iter())
            .map(|s| s.to_lowercase())
    }
}

/// WordNet descriptions mix gloss fragments with quoted usage examples.
pub fn split_desc(desc: &[String]) -> (String, Vec<String>) {
    let mut gloss = Vec::new();
    let mut examples = Vec::new();
    for d in desc {
        let t = d.trim();
        if t.len() >= 2 && t.starts_with('"') {
            examples.push(t.trim_matches('"').to_string());
        } else if !t.is_empty() {
            gloss.push(t);
        }
    }
    (gloss.join("; "), examples)
}

/// One meaning on a thesaurus page.
#[derive(Debug, Clone, Serialize)]
pub struct Sense {
    pub key: String,
    pub pos: Pos,
    /// The word this sense is filed under in the source data.
    pub headword: String,
    pub gloss: String,
    pub examples: Vec<String>,
    /// Synonyms for the page's word in this sense.
    pub synonyms: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Page {
    pub word: String,
    pub senses: Vec<Sense>,
}

impl Page {
    pub fn is_empty(&self) -> bool {
        self.senses.is_empty()
    }

    pub fn parts_of_speech(&self) -> Vec<Pos> {
        let mut v: Vec<Pos> = self.senses.iter().map(|s| s.pos).collect();
        v.sort();
        v.dedup();
        v
    }
}

/// A fuzzy search result.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub word: String,
    pub pos: Vec<Pos>,
    pub senses: u64,
    /// Gloss of the lemma's first sense, for previews.
    pub gloss: String,
    pub score: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filter {
    pub pos: Option<Pos>,
    pub key: Option<String>,
}

/// A parsed query: free text plus optional `pos:<pos>` / `key:<key>` terms.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub text: String,
    pub filter: Filter,
}

impl Query {
    pub fn parse(input: &str) -> Result<Query, String> {
        let mut q = Query::default();
        let mut words = Vec::new();
        for tok in input.split_whitespace() {
            match tok.split_once(':') {
                Some(("pos" | "p", v)) if !v.is_empty() => q.filter.pos = Some(v.parse()?),
                Some(("key" | "k", v)) if !v.is_empty() => q.filter.key = Some(v.to_string()),
                _ => words.push(tok),
            }
        }
        q.text = words.join(" ");
        Ok(q)
    }
}

/// Lowercase and join whitespace with underscores, matching WordNet lemmas.
pub fn normalize(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .to_lowercase()
}

/// Human-readable form of a lemma.
pub fn display(s: &str) -> String {
    s.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filters() {
        let q = Query::parse("happy pos:adj").unwrap();
        assert_eq!(q.text, "happy");
        assert_eq!(q.filter.pos, Some(Pos::Adj));
        let q = Query::parse("key:happy_2").unwrap();
        assert_eq!(q.text, "");
        assert_eq!(q.filter.key.as_deref(), Some("happy_2"));
        assert!(Query::parse("x pos:zzz").is_err());
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("  Ice  Cream "), "ice_cream");
        assert_eq!(display("ice_cream"), "ice cream");
    }

    #[test]
    fn splits_desc() {
        let (g, e) = split_desc(&[
            "showing joy".into(),
            "especially made happy".into(),
            "\"glad you are here\"".into(),
        ]);
        assert_eq!(g, "showing joy; especially made happy");
        assert_eq!(e, vec!["glad you are here"]);
    }
}
