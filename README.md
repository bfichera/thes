# thes

A lightning-fast offline thesaurus for the terminal.

## Install

macOS and Linux:

```sh
curl -fsSL https://github.com/bfichera/thes/releases/latest/download/install.sh | sh
```

This installs to `~/.local/bin` (set `THES_INSTALL_DIR` to change it, or
`THES_VERSION=0.2.0` to pin a version).

The WordNet data ships inside the binary, so `thes` works offline right away. On
first run it builds its database and index in about a second.

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
| `j` / `k` | previous / next sense (or word, when the word list is focused); take a count: `10j`, `5k` |
| `l` / `→` | focus the word list on the right |
| `h` / `←`, `Esc` | back to the sense list |
| `0` / `$`, `g` / `G` | first / last word; first / last sense (`3G` jumps to the 3rd) |
| `^d` / `^u` | half a page down / up |
| `Enter` | open the selected word's page (in the word list) |
| `^o` / `Backspace`, `Tab` | back, forward (jumplist) |
| `y` / `Y` | yank word / page word |
| `/` or `s` | search |
| `f` | cycle part-of-speech filter |
| `R` | reset: clear history and return to the start screen |
| `?`, `q` | help, quit |

| search | |
|---|---|
| type | live fuzzy search |
| `^j` / `^k`, `↑` / `↓` | move |
| `Enter` | open |
| `Esc` | normal mode (`j`/`k`, `i` to type, `R` reset, `q` quit) |
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
en_thesaurus.jsonl   downloaded source (only with --refresh/--source URL)
```

Use another source in the same format with `thes build --source <path-or-url>`
or `THES_SOURCE`. `thes build --refresh` downloads the latest upstream jsonl.
Shell completions: `thes completions <bash|zsh|fish>`.

A word's page lists every synset that contains it, including senses filed under a
different headword (e.g. `happy` shows the `glad_3` sense where it appears as a synonym).

## How it works

- **Data**: [zaibacu/thesaurus](https://github.com/zaibacu/thesaurus) (WordNet 3.0, ~118k senses, ~148k words), WordNet relations (similar-to, see-also, broader/narrower) for the `related` row, and the public-domain Moby Thesaurus II, filtered per sense through WordNet (see `src/more.rs`), for the `more` row
- **Storage**: embedded [Turso](https://github.com/tursodatabase/turso) database (`thes.db`)
- **Search**: [Tantivy](https://github.com/quickwit-oss/tantivy) fuzzy/prefix index
- **UI**: [ratatui](https://ratatui.rs) browser with vim keys

## Build from source

Requires Rust 1.88 or newer.

```sh
git clone https://github.com/bfichera/thes && cd thes
cargo install --path .
```

To build without the embedded data (smaller binary that downloads the
data with curl on first run), use `cargo install --path . --no-default-features`.

## License

Code: MIT ([LICENSE](LICENSE)). Data: WordNet 3.0 license, Princeton University ([LICENSE-WORDNET](LICENSE-WORDNET)); Moby Thesaurus II is public domain.
