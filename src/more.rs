//! Sense-aware extra words: Moby Thesaurus links, filtered through WordNet.
//!
//! Moby has no senses or parts of speech. For each WordNet sense we take its
//! members (plus nearby synsets' members when it is small) as anchors, let every
//! Moby word vote by how many anchors it links to, and keep words that WordNet
//! also lists with the sense's part of speech.

use crate::model::{Entry, Pos};
use std::collections::{HashMap, HashSet};

/// Most extra words kept per sense.
pub const MAX_MORE: usize = 40;

/// Strip WordNet's `(a)` / `(p)` / `(ip)` markers, keeping case.
pub fn strip_marker(lemma: &str) -> &str {
    match lemma.strip_suffix(')').and_then(|s| s.rsplit_once('(')) {
        Some((head, tag)) if !head.is_empty() && tag.chars().all(|c| c.is_ascii_alphabetic()) => head,
        _ => lemma,
    }
}

/// Lowercase lemma without markers.
pub fn clean(lemma: &str) -> String {
    strip_marker(lemma).to_lowercase()
}

/// Undirected Moby graph over lemma strings.
pub struct Moby {
    ids: HashMap<String, u32>,
    names: Vec<String>,
    adj: Vec<Vec<u32>>,
}

impl Moby {
    /// Parse `headword,word,word,...` lines.
    pub fn parse(text: &str) -> Moby {
        let mut m = Moby { ids: HashMap::new(), names: Vec::new(), adj: Vec::new() };
        for line in text.lines() {
            let mut it = line.split(',').map(|w| w.trim().to_lowercase().replace(' ', "_"));
            let Some(head) = it.next().filter(|h| !h.is_empty()) else { continue };
            let h = m.id(&head);
            for w in it.filter(|w| !w.is_empty() && *w != head) {
                let w = m.id(&w);
                m.adj[h as usize].push(w);
                m.adj[w as usize].push(h);
            }
        }
        for a in &mut m.adj {
            a.sort_unstable();
            a.dedup();
        }
        m
    }

    fn id(&mut self, w: &str) -> u32 {
        if let Some(&i) = self.ids.get(w) {
            return i;
        }
        let i = self.names.len() as u32;
        self.ids.insert(w.to_string(), i);
        self.names.push(w.to_string());
        self.adj.push(Vec::new());
        i
    }
}

fn bit(p: Pos) -> u8 {
    1 << (p as u8)
}

/// Extra lemmas (lowercase, underscored) for each entry, best first.
///
/// `nbrs[i]` are the entries linked to entry `i` by any WordNet relation;
/// `shown[i]` are lemmas already displayed for it, which are skipped.
pub fn compute(
    entries: &[Entry],
    nbrs: &[Vec<usize>],
    shown: &[HashSet<String>],
    moby: &Moby,
) -> Vec<Vec<String>> {
    let members: Vec<Vec<String>> = entries
        .iter()
        .map(|e| {
            let mut v: Vec<String> = e.lemmas().map(|l| clean(&l)).collect();
            v.sort();
            v.dedup();
            v
        })
        .collect();

    let mut pos_of: HashMap<&str, u8> = HashMap::new();
    for (e, ms) in entries.iter().zip(&members) {
        for m in ms {
            *pos_of.entry(m).or_default() |= bit(e.pos);
        }
    }
    let cand_pos: Vec<u8> =
        moby.names.iter().map(|n| pos_of.get(n.as_str()).copied().unwrap_or(0)).collect();

    entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut anchors: HashMap<&str, f32> = members[i].iter().map(|m| (m.as_str(), 1.0)).collect();
            if anchors.len() < 3 {
                for &j in &nbrs[i] {
                    for m in &members[j] {
                        anchors.entry(m.as_str()).or_insert(0.5);
                    }
                }
            }
            let total: f32 = anchors.values().sum();
            if total < 3.0 {
                return Vec::new();
            }
            let mut votes: HashMap<u32, f32> = HashMap::new();
            for (a, w) in &anchors {
                if let Some(&id) = moby.ids.get(*a) {
                    for &c in &moby.adj[id as usize] {
                        *votes.entry(c).or_default() += w;
                    }
                }
            }
            let need = if total >= 5.0 { 2.0 } else { (0.6 * total).max(3.0) };
            let mut out: Vec<(f32, &str)> = votes
                .into_iter()
                .filter(|&(c, v)| v >= need && cand_pos[c as usize] & bit(e.pos) != 0)
                .map(|(c, v)| (v, moby.names[c as usize].as_str()))
                .filter(|(_, n)| !anchors.contains_key(n) && !shown[i].contains(*n))
                .collect();
            out.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
            out.into_iter().take(MAX_MORE).map(|(_, n)| n.to_string()).collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, pos: Pos, word: &str, syn: &[&str]) -> Entry {
        Entry {
            key: key.into(),
            word: word.into(),
            pos,
            wordnet_id: String::new(),
            gloss: String::new(),
            examples: vec![],
            synonyms: syn.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn cleans_markers() {
        assert_eq!(clean("Chief(a)"), "chief");
        assert_eq!(clean("Ice_Cream"), "ice_cream");
    }

    #[test]
    fn votes_filters_by_pos_and_skips_shown() {
        let moby = Moby::parse(
            "big,large,great,bank\nlarge,huge,mighty,great\nhuge,vast,mighty\nvast,mighty\ngrand,bank\n",
        );
        let entries = vec![
            entry("big_1", Pos::Adj, "big", &["large", "huge", "vast", "grand"]),
            entry("mighty_1", Pos::Adj, "mighty", &[]),
            entry("great_1", Pos::Adj, "great", &[]),
            entry("bank_1", Pos::Noun, "bank", &[]),
        ];
        let nbrs = vec![Vec::new(); entries.len()];
        let mut shown = vec![HashSet::new(); entries.len()];
        let r = compute(&entries, &nbrs, &shown, &moby);
        assert_eq!(r[0], ["mighty", "great"]);
        shown[0].insert("great".to_string());
        let r = compute(&entries, &nbrs, &shown, &moby);
        assert_eq!(r[0], ["mighty"]);
        assert!(r[1].is_empty());
    }
}
