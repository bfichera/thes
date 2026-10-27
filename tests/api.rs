use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use thes::{Filter, Paths, Pos, Query, Thesaurus, ingest};

fn built() -> &'static Paths {
    static P: OnceLock<Paths> = OnceLock::new();
    P.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("thes-test");
        let _ = std::fs::remove_dir_all(&root);
        let paths = Paths::new(&root);
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixture.jsonl");
        let r = ingest::build(&paths, &src, &|_| {}).unwrap();
        assert_eq!(r.senses, 8);
        paths
    })
}

fn open() -> Thesaurus {
    Thesaurus::open(built()).unwrap()
}

#[test]
fn page_includes_senses_where_word_is_a_synonym() {
    let t = open();
    let page = t.page("happy", &Filter::default()).unwrap();
    let keys: Vec<_> = page.senses.iter().map(|s| s.key.as_str()).collect();
    // Own senses first, then senses where it appears as a synonym.
    assert_eq!(keys, ["happy_1", "happy_2", "felicitous_2", "glad_3"]);
    let glad = &page.senses[3];
    assert_eq!(glad.synonyms, ["glad"]);
    assert_eq!(glad.examples, ["glad to help"]);
}

#[test]
fn synonyms_link_back() {
    let t = open();
    let glad = t.page("glad", &Filter::default()).unwrap();
    assert_eq!(glad.senses[0].synonyms, ["happy"]);
    let happy = t.page(&glad.senses[0].synonyms[0], &Filter::default()).unwrap();
    assert!(happy.senses.iter().any(|s| s.synonyms.contains(&"glad".to_string())));
}

#[test]
fn filters_by_pos_and_key() {
    let t = open();
    let verb = t.page("run", &Filter { pos: Some(Pos::Verb), key: None }).unwrap();
    assert_eq!(verb.senses.len(), 1);
    assert_eq!(verb.senses[0].synonyms, ["race"]);
    let k = t.page("run", &Filter { pos: None, key: Some("run_2".into()) }).unwrap();
    assert_eq!(k.senses[0].gloss.split(' ').next(), Some("a"));
    assert_eq!(t.sense("glad_3").unwrap().unwrap().headword, "glad");
}

#[test]
fn self_synonyms_are_dropped_and_case_preserved() {
    let t = open();
    let ic = t.page("Ice Cream", &Filter::default()).unwrap();
    assert_eq!(ic.word, "ice_cream");
    assert_eq!(ic.senses[0].synonyms, ["icecream"]);
    let paris = t.page("paris", &Filter::default()).unwrap();
    assert_eq!(paris.word, "Paris");
    let col = t.page("city of light", &Filter::default()).unwrap();
    assert!(col.senses[0].synonyms.contains(&"Paris".to_string()));
}

#[test]
fn fuzzy_search() {
    let t = open();
    let hits = t.search(&Query::parse("hapy").unwrap(), 5).unwrap();
    assert_eq!(hits[0].word, "happy");
    let hits = t.search(&Query::parse("cream").unwrap(), 5).unwrap();
    assert!(hits.iter().any(|h| h.word == "ice_cream"));
    let hits = t.search(&Query::parse("ru pos:noun").unwrap(), 5).unwrap();
    assert!(hits.iter().all(|h| h.pos.contains(&Pos::Noun)));
    assert_eq!(hits[0].word, "run");
    let hits = t.search(&Query::parse("key:glad_3").unwrap(), 5).unwrap();
    let mut words: Vec<_> = hits.iter().map(|h| h.word.as_str()).collect();
    words.sort();
    assert_eq!(words, ["glad", "happy"]);
}
