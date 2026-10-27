mod print;
mod tui;

use anyhow::{Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use std::io::IsTerminal;
use thes::{Filter, Paths, Pos, Query, Thesaurus, ingest};

/// A lightning-fast terminal thesaurus.
///
/// With no arguments, opens the interactive browser. With a word, opens it
/// directly (or prints it when stdout is not a terminal or with --print).
#[derive(Parser)]
#[command(name = "thes", version, args_conflicts_with_subcommands = true)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,

    /// Word to look up. Use `thes -- build` to look up a word named like a subcommand.
    word: Vec<String>,

    /// Print the thesaurus page instead of opening the browser.
    #[arg(short, long)]
    print: bool,

    #[command(flatten)]
    filter: FilterArgs,

    /// Emit JSON (implies --print).
    #[arg(long)]
    json: bool,
}

#[derive(Args, Clone, Default)]
struct FilterArgs {
    /// Only senses with this part of speech: noun, verb, adj, adv.
    #[arg(long, short = 'P', value_parser = parse_pos, global = true)]
    pos: Option<Pos>,

    /// Only the sense with this key (e.g. happy_2).
    #[arg(long, short = 'k', global = true)]
    key: Option<String>,
}

impl FilterArgs {
    fn merge(&self, f: &Filter) -> Filter {
        Filter {
            pos: self.pos.or(f.pos),
            key: self.key.clone().or_else(|| f.key.clone()),
        }
    }
}

fn parse_pos(s: &str) -> Result<Pos, String> {
    s.parse()
}

#[derive(Subcommand)]
enum Cmd {
    /// Fuzzy-search words. Accepts inline `pos:adj` and `key:happy_2` filters.
    #[command(alias = "s")]
    Search {
        query: Vec<String>,
        /// Maximum results.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        json: bool,
    },
    /// Print the thesaurus page for an exact word (no TUI).
    Show {
        word: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// (Re)build the database and search index.
    ///
    /// Uses the data embedded in the binary unless --source is given.
    Build {
        /// Path (.jsonl or .jsonl.zst) or URL in zaibacu/thesaurus format, or `embedded`.
        #[arg(long)]
        source: Option<String>,
        /// Download the latest upstream jsonl instead of using embedded data.
        #[arg(long, conflicts_with = "source")]
        refresh: bool,
    },
    /// Show where data lives and whether it is built.
    Info,
    /// Print shell completions to stdout.
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

fn log(msg: &str) {
    eprintln!("thes: {msg}");
}

fn main() {
    if let Err(e) = run() {
        eprintln!("thes: error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::from_env()?;
    let tty = std::io::stdout().is_terminal() && std::io::stdin().is_terminal();

    match cli.cmd {
        Some(Cmd::Build { source, refresh }) => {
            let source = if refresh {
                let _ = std::fs::remove_file(paths.source_cache());
                Some(thes::paths::DEFAULT_SOURCE.to_string())
            } else {
                source
            };
            let src = ingest::resolve_source(&paths, source.as_deref(), &log)?;
            let r = ingest::build(&paths, &src, &log)?;
            log(&format!(
                "built {} senses / {} words in {:.1}s ({} skipped) -> {}",
                r.senses,
                r.lemmas,
                r.millis as f64 / 1000.0,
                r.skipped,
                paths.root.display()
            ));
            Ok(())
        }
        Some(Cmd::Info) => {
            println!("data:     {}", paths.root.display());
            println!("db:       {}", paths.db().display());
            println!("index:    {}", paths.index().display());
            println!("version:  {}", env!("CARGO_PKG_VERSION"));
            println!("embedded: {}", if ingest::HAS_EMBEDDED { "yes" } else { "no" });
            if paths.is_current() {
                let t = Thesaurus::open(&paths)?;
                println!("senses:   {}", t.meta("senses")?.unwrap_or_default());
                println!("source:   {}", t.meta("source")?.unwrap_or_default());
            } else {
                println!("status:   not built (builds automatically on first use)");
            }
            Ok(())
        }
        Some(Cmd::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "thes", &mut std::io::stdout());
            Ok(())
        }
        Some(Cmd::Search { query, limit, json }) => {
            let t = Thesaurus::open_or_build(&paths, &log)?;
            let mut q = Query::parse(&query.join(" ")).map_err(anyhow::Error::msg)?;
            q.filter = cli.filter.merge(&q.filter);
            let hits = t.search(&q, limit)?;
            if json {
                print::json(&hits)
            } else {
                print::hits(&hits)
            }
        }
        Some(Cmd::Show { word, json }) => {
            let t = Thesaurus::open_or_build(&paths, &log)?;
            show(&t, &word.join(" "), &cli.filter, json)
        }
        None => {
            let t = Thesaurus::open_or_build(&paths, &log)?;
            let input = cli.word.join(" ");
            if cli.print || cli.json || !tty {
                if input.trim().is_empty() {
                    bail!("no word given");
                }
                show(&t, &input, &cli.filter, cli.json)
            } else {
                tui::run(t, &input, cli.filter.pos)
            }
        }
    }
}

fn show(t: &Thesaurus, input: &str, filter: &FilterArgs, json: bool) -> Result<()> {
    let q = Query::parse(input).map_err(anyhow::Error::msg)?;
    let filter = filter.merge(&q.filter);
    if q.text.is_empty() {
        let Some(key) = &filter.key else {
            bail!("no word given");
        };
        let Some(sense) = t.sense(key)? else {
            bail!("no sense with key '{key}'");
        };
        return if json { print::json(&sense) } else { print::sense(&sense) };
    }
    let page = t.page(&q.text, &filter)?;
    if page.is_empty() {
        let hits = t.search(&Query { text: q.text.clone(), filter }, 8)?;
        if json {
            print::json(&page)?;
        } else {
            print::not_found(&q.text, &hits)?;
        }
        std::process::exit(2);
    }
    if json { print::json(&page) } else { print::page(&page) }
}
