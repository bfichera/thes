use anyhow::Result;
use crossterm::style::Stylize;
use serde::Serialize;
use std::io::{IsTerminal, Write};
use thes::{Hit, Page, Sense, display};

fn color() -> bool {
    std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}

macro_rules! paint {
    ($s:expr, $($m:ident),+) => {{
        let s: &str = &$s;
        if color() { s.$($m()).+.to_string() } else { s.to_string() }
    }};
}

pub fn json<T: Serialize>(v: &T) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, v)?;
    writeln!(out)?;
    Ok(())
}

pub fn hits(hits: &[Hit]) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let w = hits.iter().map(|h| h.word.chars().count()).max().unwrap_or(0);
    for h in hits {
        let pos: Vec<_> = h.pos.iter().map(|p| p.as_str()).collect();
        let word = display(&h.word);
        let pad = " ".repeat(w.saturating_sub(word.chars().count()));
        writeln!(
            out,
            "{}{pad}  {:<15} {}",
            paint!(word, bold),
            paint!(pos.join(","), dark_yellow),
            paint!(truncate(&h.gloss, 70), dim)
        )?;
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n - 1).collect::<String>())
    }
}

fn write_sense(out: &mut impl Write, i: usize, s: &Sense) -> Result<()> {
    writeln!(
        out,
        "  {} {} {}",
        paint!(format!("{:>2}.", i + 1), dim),
        paint!(format!("{:<4}", s.pos.as_str()), dark_yellow),
        s.gloss
    )?;
    for ex in &s.examples {
        writeln!(out, "           {}", paint!(format!("\"{ex}\""), dim, italic))?;
    }
    if !s.synonyms.is_empty() {
        let syns: Vec<_> = s.synonyms.iter().map(|x| display(x)).collect();
        writeln!(out, "           {}", paint!(syns.join(", "), cyan))?;
    }
    writeln!(out, "           {}", paint!(s.key, dark_grey))?;
    Ok(())
}

pub fn page(p: &Page) -> Result<()> {
    let mut out = std::io::stdout().lock();
    let pos: Vec<_> = p.parts_of_speech().iter().map(|p| p.long()).collect();
    writeln!(
        out,
        "{}  {}",
        paint!(display(&p.word), bold),
        paint!(pos.join(" · "), dark_yellow)
    )?;
    for (i, s) in p.senses.iter().enumerate() {
        write_sense(&mut out, i, s)?;
    }
    Ok(())
}

pub fn sense(s: &Sense) -> Result<()> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", paint!(display(&s.headword), bold))?;
    write_sense(&mut out, 0, s)
}

pub fn not_found(word: &str, hits: &[Hit]) -> Result<()> {
    let mut err = std::io::stderr().lock();
    writeln!(err, "no entry for '{word}'")?;
    if !hits.is_empty() {
        let s: Vec<_> = hits.iter().map(|h| display(&h.word)).collect();
        writeln!(err, "did you mean: {}", s.join(", "))?;
    }
    Ok(())
}
