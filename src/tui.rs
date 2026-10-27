use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use thes::{Filter, Hit, Page, Pos, Query, Thesaurus, display};
use unicode_width::UnicodeWidthStr;

const ACCENT: Color = Color::Rgb(255, 135, 0);
const CHIP: Color = Color::Rgb(130, 190, 255);
const DIM: Color = Color::Rgb(120, 120, 120);
const FAINT: Color = Color::Rgb(80, 80, 80);
const MAX_WIDTH: u16 = 100;
const SEARCH_LIMIT: usize = 100;

pub fn run(t: Thesaurus, initial: &str, pos: Option<Pos>) -> Result<()> {
    let mut app = App::new(t, pos);
    app.start(initial)?;
    let mut terminal = ratatui::init();
    let res = app.run(&mut terminal);
    ratatui::restore();
    res
}

#[derive(Clone)]
struct Visit {
    word: String,
    sense: usize,
    syn: usize,
}

struct PageView {
    page: Page,
    sense: usize,
    syn: usize,
    scroll: usize,
}

impl PageView {
    fn selectable(&self, i: usize) -> bool {
        self.page.senses.get(i).is_some_and(|s| !s.synonyms.is_empty())
    }

    fn first(&self) -> Option<usize> {
        (0..self.page.senses.len()).find(|&i| self.selectable(i))
    }

    fn last(&self) -> Option<usize> {
        (0..self.page.senses.len()).rev().find(|&i| self.selectable(i))
    }

    fn clamp(&mut self) {
        if !self.selectable(self.sense) {
            let next = (self.sense..self.page.senses.len()).find(|&i| self.selectable(i));
            match next.or_else(|| self.last()) {
                Some(i) => self.sense = i,
                None => {
                    self.sense = 0;
                    self.syn = 0;
                    return;
                }
            }
        }
        let n = self.page.senses[self.sense].synonyms.len();
        self.syn = self.syn.min(n.saturating_sub(1));
    }

    fn selected(&self) -> Option<&str> {
        self.page
            .senses
            .get(self.sense)
            .and_then(|s| s.synonyms.get(self.syn))
            .map(String::as_str)
    }

    fn move_sense(&mut self, delta: isize) {
        let mut i = self.sense;
        let mut moved = 0;
        let steps = delta.unsigned_abs();
        loop {
            let next = if delta > 0 {
                (i + 1..self.page.senses.len()).find(|&j| self.selectable(j))
            } else {
                (0..i).rev().find(|&j| self.selectable(j))
            };
            match next {
                Some(j) => {
                    i = j;
                    moved += 1;
                    if moved == steps {
                        break;
                    }
                }
                None => break,
            }
        }
        if i != self.sense {
            self.sense = i;
            self.clamp();
        }
    }

    fn move_syn(&mut self, delta: isize) {
        let Some(s) = self.page.senses.get(self.sense) else { return };
        if s.synonyms.is_empty() {
            return;
        }
        if delta > 0 {
            if self.syn + 1 < s.synonyms.len() {
                self.syn += 1;
            } else if let Some(j) =
                (self.sense + 1..self.page.senses.len()).find(|&j| self.selectable(j))
            {
                self.sense = j;
                self.syn = 0;
            }
        } else if self.syn > 0 {
            self.syn -= 1;
        } else if let Some(j) = (0..self.sense).rev().find(|&j| self.selectable(j)) {
            self.sense = j;
            self.syn = self.page.senses[j].synonyms.len() - 1;
        }
    }
}

#[derive(PartialEq)]
enum View {
    Search,
    Page,
}

struct App {
    t: Thesaurus,
    view: View,
    input: String,
    insert: bool,
    hits: Vec<Hit>,
    hit: usize,
    hit_offset: usize,
    page: Option<PageView>,
    history: Vec<Visit>,
    at: usize,
    pos: Option<Pos>,
    status: Option<(String, bool)>,
    help: bool,
    quit: bool,
}

impl App {
    fn new(t: Thesaurus, pos: Option<Pos>) -> App {
        App {
            t,
            view: View::Search,
            input: String::new(),
            insert: true,
            hits: Vec::new(),
            hit: 0,
            hit_offset: 0,
            page: None,
            history: Vec::new(),
            at: 0,
            pos,
            status: None,
            help: false,
            quit: false,
        }
    }

    fn start(&mut self, initial: &str) -> Result<()> {
        let initial = initial.trim();
        if initial.is_empty() {
            return Ok(());
        }
        if !self.open(initial)? {
            self.input = initial.to_string();
            self.refresh_search();
            self.view = View::Search;
        }
        Ok(())
    }

    fn run(&mut self, term: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            term.draw(|f| self.draw(f))?;
            if let Event::Key(k) = event::read()?
                && k.kind == KeyEventKind::Press
                && let Err(e) = self.key(k)
            {
                self.status = Some((format!("{e:#}"), true));
            }
        }
        Ok(())
    }

    fn filter(&self) -> Filter {
        Filter { pos: self.pos, key: None }
    }

    fn refresh_search(&mut self) {
        self.hit = 0;
        self.hit_offset = 0;
        match Query::parse(&self.input) {
            Err(e) => {
                self.hits.clear();
                self.status = Some((e, true));
            }
            Ok(mut q) => {
                self.status = None;
                q.filter.pos = q.filter.pos.or(self.pos);
                if q.text.is_empty() && q.filter == Filter::default() {
                    self.hits.clear();
                    return;
                }
                match self.t.search(&q, SEARCH_LIMIT) {
                    Ok(h) => self.hits = h,
                    Err(e) => self.status = Some((format!("{e:#}"), true)),
                }
            }
        }
    }

    fn save_selection(&mut self) {
        if let (Some(p), Some(v)) = (&self.page, self.history.get_mut(self.at)) {
            v.sense = p.sense;
            v.syn = p.syn;
        }
    }

    /// Load a page for `visit`. Returns false if the word has no entries.
    fn load(&mut self, visit: &Visit) -> Result<bool> {
        let mut page = self.t.page(&visit.word, &self.filter())?;
        if let (true, Some(pos)) = (page.is_empty(), self.pos) {
            page = self.t.page(&visit.word, &Filter::default())?;
            if !page.is_empty() {
                self.status = Some((format!("no {} senses; showing all", pos.long()), false));
            }
        }
        if page.is_empty() {
            return Ok(false);
        }
        let mut pv = PageView { page, sense: visit.sense, syn: visit.syn, scroll: 0 };
        if visit.sense == usize::MAX {
            pv.sense = pv.first().unwrap_or(0);
            pv.syn = 0;
        }
        pv.clamp();
        self.page = Some(pv);
        self.view = View::Page;
        Ok(true)
    }

    fn open(&mut self, word: &str) -> Result<bool> {
        let visit = Visit { word: word.to_string(), sense: usize::MAX, syn: 0 };
        self.status = None;
        if !self.load(&visit)? {
            self.status = Some((format!("no entry for '{}'", display(word)), true));
            return Ok(false);
        }
        self.save_selection();
        if !self.history.is_empty() {
            self.history.truncate(self.at + 1);
        }
        self.history.push(visit);
        self.at = self.history.len() - 1;
        Ok(true)
    }

    fn jump(&mut self, delta: isize) -> Result<()> {
        let target = self.at as isize + delta;
        if target < 0 || target as usize >= self.history.len() {
            self.status = Some((
                if delta < 0 { "at oldest page" } else { "at newest page" }.into(),
                false,
            ));
            return Ok(());
        }
        self.save_selection();
        self.at = target as usize;
        let visit = self.history[self.at].clone();
        self.status = None;
        self.load(&visit)?;
        Ok(())
    }

    fn reload(&mut self) -> Result<()> {
        self.save_selection();
        if let Some(v) = self.history.get(self.at).cloned() {
            self.load(&v)?;
        }
        Ok(())
    }

    fn cycle_pos(&mut self) -> Result<()> {
        self.pos = match self.pos {
            None => Some(Pos::Noun),
            Some(Pos::Noun) => Some(Pos::Verb),
            Some(Pos::Verb) => Some(Pos::Adj),
            Some(Pos::Adj) => Some(Pos::Adv),
            Some(Pos::Adv) => None,
        };
        self.status = None;
        match self.view {
            View::Page => self.reload(),
            View::Search => {
                self.refresh_search();
                Ok(())
            }
        }
    }

    fn key(&mut self, k: KeyEvent) -> Result<()> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            self.quit = true;
            return Ok(());
        }
        if self.help {
            self.help = false;
            return Ok(());
        }
        match self.view {
            View::Search => self.search_key(k, ctrl),
            View::Page => self.page_key(k, ctrl),
        }
    }

    fn move_hit(&mut self, delta: isize) {
        if self.hits.is_empty() {
            return;
        }
        let n = self.hits.len() as isize;
        self.hit = (self.hit as isize + delta).clamp(0, n - 1) as usize;
    }

    fn open_hit(&mut self) -> Result<()> {
        let q = Query::parse(&self.input).unwrap_or_default();
        let word = match self.hits.get(self.hit) {
            Some(h) => h.word.clone(),
            None if !q.text.is_empty() => q.text.clone(),
            None => return Ok(()),
        };
        if q.filter.pos.is_some() {
            self.pos = q.filter.pos;
        }
        self.open(&word)?;
        Ok(())
    }

    fn search_key(&mut self, k: KeyEvent, ctrl: bool) -> Result<()> {
        use KeyCode::*;
        match (k.code, ctrl) {
            (Enter, _) => return self.open_hit(),
            (Down, _) | (Char('n' | 'j'), true) => self.move_hit(1),
            (Up, _) | (Char('p' | 'k'), true) => self.move_hit(-1),
            (Char('d'), true) => self.move_hit(10),
            (Char('u'), true) if !self.insert => self.move_hit(-10),
            (Char('o'), true) => {
                if self.page.is_some() {
                    self.view = View::Page;
                }
            }
            _ if self.insert => match (k.code, ctrl) {
                (Esc, _) => self.insert = false,
                (Backspace, _) | (Char('h'), true) => {
                    self.input.pop();
                    self.refresh_search();
                }
                (Char('w'), true) => {
                    let t = self.input.trim_end().len();
                    let cut = self.input[..t].rfind(' ').map(|i| i + 1).unwrap_or(0);
                    self.input.truncate(cut);
                    self.refresh_search();
                }
                (Char('u'), true) => {
                    self.input.clear();
                    self.refresh_search();
                }
                (Tab, _) => self.cycle_pos()?,
                (Char(c), false) => {
                    self.input.push(c);
                    self.refresh_search();
                }
                _ => {}
            },
            (Char('j'), _) => self.move_hit(1),
            (Char('k'), _) => self.move_hit(-1),
            (Char('g'), _) => self.hit = 0,
            (Char('G'), _) => self.hit = self.hits.len().saturating_sub(1),
            (Char('l'), _) => return self.open_hit(),
            (Char('i' | 'a' | '/'), _) => self.insert = true,
            (Char('S'), _) | (Char('c'), _) => {
                self.input.clear();
                self.refresh_search();
                self.insert = true;
            }
            (Char('f') | Tab, _) => self.cycle_pos()?,
            (Char('?'), _) => self.help = true,
            (Char('q'), _) => self.quit = true,
            (Esc, _) | (Char('h'), _) if self.page.is_some() => self.view = View::Page,
            _ => {}
        }
        Ok(())
    }

    fn page_key(&mut self, k: KeyEvent, ctrl: bool) -> Result<()> {
        use KeyCode::*;
        let Some(p) = self.page.as_mut() else {
            self.view = View::Search;
            return Ok(());
        };
        match (k.code, ctrl) {
            (Char('o'), true) | (Backspace, _) | (Char('-'), false) => self.jump(-1)?,
            (Char('i'), true) | (Tab, _) => self.jump(1)?,
            (Char('d'), true) => p.move_sense(5),
            (Char('u'), true) => p.move_sense(-5),
            (Char('j'), _) | (Down, _) | (Char('n'), true) => p.move_sense(1),
            (Char('k'), _) | (Up, _) | (Char('p'), true) => p.move_sense(-1),
            (Char('l' | 'w'), _) | (Right, _) => p.move_syn(1),
            (Char('h' | 'b'), _) | (Left, _) => p.move_syn(-1),
            (Char('0' | '^'), _) | (Home, _) => p.syn = 0,
            (Char('$'), _) | (End, _) => {
                if let Some(s) = p.page.senses.get(p.sense) {
                    p.syn = s.synonyms.len().saturating_sub(1);
                }
            }
            (Char('g'), _) => {
                p.sense = p.first().unwrap_or(0);
                p.syn = 0;
            }
            (Char('G'), _) => {
                p.sense = p.last().unwrap_or(0);
                p.syn = 0;
            }
            (Enter, _) => {
                if let Some(w) = p.selected().map(str::to_string) {
                    self.open(&w)?;
                }
            }
            (Char('y'), _) => {
                if let Some(w) = p.selected().map(display) {
                    yank(&w);
                    self.status = Some((format!("yanked '{w}'"), false));
                }
            }
            (Char('Y'), _) => {
                let w = display(&p.page.word);
                yank(&w);
                self.status = Some((format!("yanked '{w}'"), false));
            }
            (Char('/' | 's' | 'i'), _) => {
                self.input.clear();
                self.hits.clear();
                self.insert = true;
                self.view = View::Search;
            }
            (Char('f'), _) => self.cycle_pos()?,
            (Char('?'), _) => self.help = true,
            (Char('q'), _) => self.quit = true,
            (Esc, _) => self.status = None,
            _ => {}
        }
        Ok(())
    }

    fn draw(&mut self, f: &mut Frame) {
        let area = column(f.area(), MAX_WIDTH);
        let [top, body, foot] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_title(f, top);
        match self.view {
            View::Search => self.draw_search(f, body),
            View::Page => self.draw_page(f, body),
        }
        self.draw_footer(f, foot);
        if self.help {
            draw_help(f, area);
        }
    }

    fn draw_title(&self, f: &mut Frame, area: Rect) {
        let mut spans = vec![
            Span::styled(" thes ", Style::new().fg(Color::Black).bg(ACCENT).bold()),
            Span::raw(" "),
        ];
        let crumbs: Vec<String> = self
            .history
            .iter()
            .map(|v| display(&v.word))
            .collect();
        let start = crumbs.len().saturating_sub(6);
        if start > 0 {
            spans.push(Span::styled("… › ", Style::new().fg(FAINT)));
        }
        for (i, c) in crumbs.iter().enumerate().skip(start) {
            let style = if i == self.at && self.view == View::Page {
                Style::new().fg(ACCENT).bold()
            } else {
                Style::new().fg(DIM)
            };
            spans.push(Span::styled(c.clone(), style));
            if i + 1 < crumbs.len() {
                spans.push(Span::styled(" › ", Style::new().fg(FAINT)));
            }
        }
        let pos = match self.pos {
            Some(p) => format!("pos:{} ", p.as_str()),
            None => "pos:all ".into(),
        };
        let [left, right] =
            Layout::horizontal([Constraint::Min(1), Constraint::Length(pos.len() as u16)])
                .areas(area);
        f.render_widget(Line::from(spans), left);
        f.render_widget(
            Line::styled(pos, Style::new().fg(if self.pos.is_some() { ACCENT } else { FAINT })),
            right,
        );
    }

    fn draw_search(&mut self, f: &mut Frame, area: Rect) {
        let [input_area, _, list] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .areas(area);
        let border = if self.insert { ACCENT } else { FAINT };
        let block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(border))
            .title(Span::styled(" search ", Style::new().fg(border)));
        let mut spans = vec![Span::styled("› ", Style::new().fg(ACCENT)), Span::raw(&self.input)];
        if self.input.is_empty() {
            spans.push(Span::styled(
                "type a word… (pos:adj, key:happy_2)",
                Style::new().fg(FAINT),
            ));
        }
        f.render_widget(Paragraph::new(Line::from(spans)).block(block), input_area);
        if self.insert {
            let x = input_area.x + 3 + self.input.width() as u16;
            f.set_cursor_position((x.min(input_area.right().saturating_sub(2)), input_area.y + 1));
        }

        let h = list.height as usize;
        if h == 0 {
            return;
        }
        if self.hits.is_empty() {
            let msg = if self.input.trim().is_empty() { "" } else { "no matches" };
            f.render_widget(Line::styled(format!("  {msg}"), Style::new().fg(DIM)), list);
            return;
        }
        if self.hit < self.hit_offset {
            self.hit_offset = self.hit;
        } else if self.hit >= self.hit_offset + h {
            self.hit_offset = self.hit + 1 - h;
        }
        let ww = self
            .hits
            .iter()
            .map(|h| display(&h.word).width())
            .max()
            .unwrap_or(0)
            .clamp(8, 28);
        let lines: Vec<Line> = self
            .hits
            .iter()
            .enumerate()
            .skip(self.hit_offset)
            .take(h)
            .map(|(i, hit)| {
                let sel = i == self.hit;
                let word = truncate(&display(&hit.word), ww);
                let pos: Vec<_> = hit.pos.iter().map(|p| p.as_str()).collect();
                let pos = format!("{:<16}", pos.join(" "));
                let rest = (list.width as usize).saturating_sub(ww + 22);
                let marker = if sel { "▌ " } else { "  " };
                let mut wstyle = Style::new().fg(Color::White);
                if sel {
                    wstyle = wstyle.fg(ACCENT).bold();
                }
                Line::from(vec![
                    Span::styled(marker, Style::new().fg(ACCENT)),
                    Span::styled(format!("{word:<ww$}  "), wstyle),
                    Span::styled(pos, Style::new().fg(if sel { CHIP } else { DIM })),
                    Span::styled(
                        truncate(&hit.gloss, rest),
                        Style::new().fg(if sel { Color::Gray } else { FAINT }),
                    ),
                ])
            })
            .collect();
        f.render_widget(Paragraph::new(lines), list);
    }

    fn draw_page(&mut self, f: &mut Frame, area: Rect) {
        let Some(p) = self.page.as_mut() else { return };
        let width = area.width as usize;
        let (lines, sense_spans, sel_line) = layout_page(p, width);
        let h = area.height as usize;

        if let Some((start, end)) = sense_spans.get(p.sense).copied() {
            let is_first = p.first() == Some(p.sense);
            let is_last = p.last() == Some(p.sense);
            let top = if is_first { 0 } else { start };
            let bottom = if is_last { lines.len() } else { end };
            if top < p.scroll {
                p.scroll = top;
            }
            if bottom > p.scroll + h && bottom - top <= h {
                p.scroll = bottom - h;
            }
            if sel_line >= p.scroll + h {
                p.scroll = sel_line + 1 - h;
            }
            if sel_line < p.scroll {
                p.scroll = sel_line;
            }
        }
        p.scroll = p.scroll.min(lines.len().saturating_sub(1));
        f.render_widget(Paragraph::new(lines).scroll((p.scroll as u16, 0)), area);
    }

    fn draw_footer(&self, f: &mut Frame, area: Rect) {
        let (left, left_style) = match &self.status {
            Some((m, true)) => (m.clone(), Style::new().fg(Color::Red)),
            Some((m, false)) => (m.clone(), Style::new().fg(ACCENT)),
            None => match self.view {
                View::Page => (
                    self.page
                        .as_ref()
                        .and_then(|p| p.page.senses.get(p.sense))
                        .map(|s| s.key.clone())
                        .unwrap_or_default(),
                    Style::new().fg(FAINT),
                ),
                View::Search => (
                    if self.insert { "INSERT" } else { "NORMAL" }.to_string(),
                    Style::new().fg(if self.insert { ACCENT } else { DIM }),
                ),
            },
        };
        let hints: &[(&str, &str)] = match (&self.view, self.insert) {
            (View::Page, _) => &[
                ("j/k", "sense"),
                ("h/l", "word"),
                ("⏎", "open"),
                ("^o/tab", "back/fwd"),
                ("/", "search"),
                ("f", "pos"),
                ("?", "help"),
            ],
            (View::Search, true) => &[
                ("^j/^k", "move"),
                ("⏎", "open"),
                ("tab", "pos"),
                ("esc", "normal"),
            ],
            (View::Search, false) => &[
                ("j/k", "move"),
                ("⏎", "open"),
                ("i", "insert"),
                ("f", "pos"),
                ("q", "quit"),
            ],
        };
        let mut spans = Vec::new();
        for (k, v) in hints {
            spans.push(Span::styled(*k, Style::new().fg(ACCENT)));
            spans.push(Span::styled(format!(" {v}  "), Style::new().fg(DIM)));
        }
        let right = Line::from(spans).right_aligned();
        let lw = (left.width() as u16 + 2).min(area.width / 2);
        let [l, r] = Layout::horizontal([Constraint::Length(lw), Constraint::Min(0)]).areas(area);
        f.render_widget(Line::styled(format!(" {left}"), left_style), l);
        f.render_widget(right, r);
    }
}

/// Lay out a page into pre-wrapped lines. Returns the lines, each sense's
/// `[start, end)` line range, and the line holding the selected synonym.
fn layout_page(p: &PageView, width: usize) -> (Vec<Line<'static>>, Vec<(usize, usize)>, usize) {
    const INDENT: usize = 7;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut spans_out = vec![(0, 0); p.page.senses.len()];
    let mut sel_line = 0;
    let body = width.saturating_sub(INDENT + 1).max(10);

    let pos: Vec<_> = p.page.parts_of_speech().iter().map(|x| x.long()).collect();
    lines.push(Line::default());
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(display(&p.page.word), Style::new().fg(ACCENT).bold()),
        Span::raw("   "),
        Span::styled(pos.join(" · "), Style::new().fg(DIM).italic()),
        Span::styled(
            match p.page.senses.len() {
                1 => "   1 sense".to_string(),
                n => format!("   {n} senses"),
            },
            Style::new().fg(FAINT),
        ),
    ]));

    let mut current_pos = None;
    let mut n = 0;
    for (i, s) in p.page.senses.iter().enumerate() {
        if current_pos != Some(s.pos) {
            current_pos = Some(s.pos);
            n = 0;
            lines.push(Line::default());
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    s.pos.long().to_string(),
                    Style::new().fg(Color::White).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                ),
            ]));
        }
        n += 1;
        let active = i == p.sense && p.selected().is_some();
        let gutter = || {
            if active {
                Span::styled("▎", Style::new().fg(ACCENT))
            } else {
                Span::raw(" ")
            }
        };
        let start = lines.len();
        lines.push(Line::from(gutter()));

        for (j, g) in wrap(&s.gloss, body).into_iter().enumerate() {
            let num = if j == 0 { format!(" {n:>3}. ") } else { " ".repeat(INDENT - 1) };
            lines.push(Line::from(vec![
                gutter(),
                Span::styled(num, Style::new().fg(if active { ACCENT } else { DIM })),
                Span::styled(g, Style::new().fg(if active { Color::White } else { Color::Gray })),
            ]));
        }
        for ex in &s.examples {
            for g in wrap(&format!("“{ex}”"), body) {
                lines.push(Line::from(vec![
                    gutter(),
                    Span::raw(" ".repeat(INDENT - 1)),
                    Span::styled(g, Style::new().fg(DIM).italic()),
                ]));
            }
        }
        if !s.synonyms.is_empty() {
            let mut row: Vec<Span<'static>> = vec![gutter(), Span::raw(" ".repeat(INDENT - 2))];
            let mut w = INDENT - 1;
            for (k, syn) in s.synonyms.iter().enumerate() {
                let label = format!(" {} ", display(syn));
                let lw = label.width();
                if w + lw > width && w > INDENT - 1 {
                    lines.push(Line::from(std::mem::take(&mut row)));
                    row = vec![gutter(), Span::raw(" ".repeat(INDENT - 2))];
                    w = INDENT - 1;
                }
                let selected = active && k == p.syn;
                let style = if selected {
                    sel_line = lines.len();
                    Style::new().fg(Color::Black).bg(ACCENT).bold()
                } else if active {
                    Style::new().fg(CHIP).underlined()
                } else {
                    Style::new().fg(CHIP)
                };
                row.push(Span::styled(label, style));
                w += lw;
            }
            lines.push(Line::from(row));
        }
        spans_out[i] = (start, lines.len());
    }
    lines.push(Line::default());
    (lines, spans_out, sel_line)
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut w = 0;
    for word in text.split_whitespace() {
        let ww = word.width();
        if w > 0 && w + 1 + ww > width {
            out.push(std::mem::take(&mut cur));
            w = 0;
        }
        if w > 0 {
            cur.push(' ');
            w += 1;
        }
        cur.push_str(word);
        w += ww;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn truncate(s: &str, n: usize) -> String {
    if s.width() <= n {
        return s.to_string();
    }
    let mut out = String::new();
    for c in s.chars() {
        if out.width() + 2 > n {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

fn column(area: Rect, max: u16) -> Rect {
    let w = area.width.min(max);
    Rect { x: area.x + (area.width - w) / 2, width: w, ..area }
}

fn draw_help(f: &mut Frame, area: Rect) {
    let rows = [
        ("page", ""),
        ("j / k", "next / previous sense"),
        ("h / l  (b / w)", "previous / next synonym"),
        ("0 / $", "first / last synonym in sense"),
        ("g / G", "first / last sense"),
        ("^d / ^u", "jump 5 senses"),
        ("enter", "open selected synonym"),
        ("^o / backspace", "back"),
        ("tab / ^i", "forward"),
        ("y / Y", "yank synonym / page word"),
        ("/  s", "search"),
        ("search", ""),
        ("type", "fuzzy search (pos:adj key:x_1)"),
        ("^j ^k  ↑ ↓", "move"),
        ("esc", "normal mode (j/k, i, q)"),
        ("^o", "return to page"),
        ("global", ""),
        ("f  (tab in search)", "cycle part-of-speech filter"),
        ("q / ^c", "quit"),
    ];
    let lines: Vec<Line> = rows
        .iter()
        .map(|(k, v)| {
            if v.is_empty() {
                Line::styled(format!(" {k}"), Style::new().fg(Color::White).bold())
            } else {
                Line::from(vec![
                    Span::styled(format!("   {k:<20}"), Style::new().fg(ACCENT)),
                    Span::styled(*v, Style::new().fg(Color::Gray)),
                ])
            }
        })
        .collect();
    let h = (lines.len() as u16 + 2).min(area.height);
    let w = 60.min(area.width);
    let r = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, r);
    f.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::new().fg(ACCENT))
                .title(" keys "),
        ),
        r,
    );
}

/// Copy to the clipboard via pbcopy/wl-copy/xclip, falling back to OSC 52.
fn yank(s: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    for (cmd, args) in [
        ("pbcopy", &[][..]),
        ("wl-copy", &[][..]),
        ("xclip", &["-selection", "clipboard"][..]),
    ] {
        if let Ok(mut child) = Command::new(cmd)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(s.as_bytes());
            }
            if child.wait().map(|s| s.success()).unwrap_or(false) {
                return;
            }
        }
    }
    let mut out = std::io::stdout();
    let _ = write!(out, "\x1b]52;c;{}\x07", base64(s.as_bytes()));
    let _ = out.flush();
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64() {
        assert_eq!(base64(b"happy"), "aGFwcHk=");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(b"abc"), "YWJj");
    }

    #[test]
    fn wraps() {
        assert_eq!(wrap("a bb ccc dddd", 6), vec!["a bb", "ccc", "dddd"]);
    }
}
