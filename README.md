# thes

A lightning-fast offline thesaurus for the terminal.

- **Data**: [zaibacu/thesaurus](https://github.com/zaibacu/thesaurus) (WordNet 3.0, ~118k senses, ~148k words)
- **Storage**: embedded [Turso](https://github.com/tursodatabase/turso) database (`thes.db`)
- **Search**: [Tantivy](https://github.com/quickwit-oss/tantivy) fuzzy/prefix index
- **UI**: [ratatui](https://ratatui.rs) browser with vim keys

## Install

```sh
git clone <this repo> thes && cd thes
cargo install --path .
thes build          # downloads the source jsonl and builds db + index (~1.5s)
```

`thes` also builds on first use if the data is missing.

## Usage

```sh
thes                      # interactive browser
thes happy                # open "happy" in the browser
thes happy -p             # print the page (also automatic when piped)
thes happy -P adj --json  # JSON, adjective senses only
thes -p key:happy_2       # a single sense by key
thes search recieve       # fuzzy search
thes search 'pos:verb run' -n 5 --json
thes info                 # where data lives
```

Queries accept inline filters anywhere: `pos:<noun|verb|adj|adv>` (also `p:`) and
`key:<sense_key>` (also `k:`). Exit status is `2` when a word has no entry.

## Keys

| page | |
|---|---|
| `j` / `k` | next / previous sense |
| `h` / `l` (`b` / `w`) | previous / next synonym (wraps across senses) |
| `0` / `$`, `g` / `G` | first / last synonym; first / last sense |
| `^d` / `^u` | jump 5 senses |
| `Enter` | open the selected synonym's page |
| `^o` / `Backspace`, `Tab` | back, forward (jumplist) |
| `y` / `Y` | yank synonym / page word |
| `/` or `s` | search |
| `f` | cycle part-of-speech filter |
| `?`, `q` | help, quit |

| search | |
|---|---|
| type | live fuzzy search |
| `^j` / `^k`, `↑` / `↓` | move |
| `Enter` | open |
| `Esc` | normal mode (`j`/`k`, `i` to type, `q` quit) |
| `Tab` | cycle part-of-speech filter |
| `^o` | return to current page |

## Library

```rust
use thes::{Thesaurus, Query, Filter, Pos};

let t = Thesaurus::open_default()?;
let hits = t.search(&Query::parse("hapy pos:adj").map_err(anyhow::Error::msg)?, 10)?;
let page = t.page("happy", &Filter { pos: Some(Pos::Adj), key: None })?;
let sense = t.sense("happy_2")?;
```

## Data

Stored in `$THES_HOME`, else `$XDG_DATA_HOME/thes`, else `~/.local/share/thes`:

```
thes.db              turso database: senses(key, word, pos, gloss, examples, lemmas), members(lemma, sid)
index/               tantivy index, one document per word
en_thesaurus.jsonl   cached source
```

Use another source in the same format with `thes build --source <path-or-url>`
or `THES_SOURCE`. `thes build --refresh` re-downloads the default.

A word's page lists every synset that contains it, including senses filed under a
different headword (e.g. `happy` shows the `glad_3` sense where it appears as a synonym).

## License

Code: MIT. Data: WordNet 3.0 license, Princeton University.
