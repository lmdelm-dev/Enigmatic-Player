use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, Btn, Mode, BANDS, NB_RV, NB_VOL, PLAYLISTS, REVERBS, SPEEDS};
use crate::library::{Provider, Track};
use crate::lyrics;
use crate::themes::{Theme, THEMES};

const MIN_W: u16 = 80;
const MIN_H: u16 = 28;
const MAX_W: u16 = 110;

fn base(t: &Theme) -> Style {
    Style::default().fg(t.text).bg(t.bg)
}

fn fg(t: &Theme, c: Color) -> Style {
    Style::default().fg(c).bg(t.bg)
}

fn trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut o: String = s.chars().take(w - 1).collect();
        o.push('…');
        o
    }
}

/// Truncates long text (share codes, invites) keeping the tail visible.
fn tail_trunc(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut o: String = s.chars().skip(s.chars().count() - (w - 1)).collect();
        o.insert(0, '…');
        o
    }
}

fn fmt_time(s: f64) -> String {
    let s = s.max(0.0) as u64;
    format!("{:02}:{:02}", s / 60, s % 60)
}

fn pad(r: Rect, n: u16) -> Rect {
    Rect::new(
        r.x + n,
        r.y,
        r.width.saturating_sub(n * 2),
        r.height,
    )
}

fn same(a: &Track, b: &Track) -> bool {
    (a.path.is_some() && a.path == b.path) || (a.url.is_some() && a.url == b.url)
}

/// Builds one line of spans and remembers where each piece landed, for mouse hits.
struct RowB {
    y: u16,
    cur: u16,
    spans: Vec<Span<'static>>,
}

impl RowB {
    fn new(x: u16, y: u16) -> Self {
        RowB { y, cur: x, spans: Vec::new() }
    }
    fn put(&mut self, s: impl Into<String>, style: Style) -> Rect {
        let s: String = s.into();
        let w = s.chars().count() as u16;
        let r = Rect::new(self.cur, self.y, w, 1);
        self.cur += w;
        self.spans.push(Span::styled(s, style));
        r
    }
    fn line(self) -> Line<'static> {
        Line::from(self.spans)
    }
}

fn art_lines(t: &Theme, seed: usize, w: usize) -> Vec<Line<'static>> {
    let accent = [t.red, t.yellow, t.purple, t.green][seed % 4];
    let mut rows: Vec<Vec<Color>> = Vec::new();
    for r in 0..6usize {
        let mut row = vec![t.panel; w];
        match r {
            1 => {
                for c in 6..10 {
                    row[c] = accent;
                }
            }
            2 => {
                for c in 5..11 {
                    row[c] = accent;
                }
            }
            3 => {
                row = vec![accent; w];
            }
            4 | 5 => {
                row = vec![t.line; w];
                let g1 = (seed / 4 + r * 3) % w;
                let g2 = (seed / 7 + r * 5 + 4) % w;
                row[g1] = t.cyan;
                row[g2] = t.cyan;
            }
            _ => {}
        }
        rows.push(row);
    }
    rows.into_iter()
        .map(|row| {
            Line::from(
                row.into_iter()
                    .map(|c| Span::styled(" ", Style::default().bg(c)))
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

fn bar_color(t: &Theme, k: usize) -> Color {
    match k {
        0..=2 => t.green,
        3..=5 => t.cyan,
        6..=8 => t.purple,
        _ => t.red,
    }
}

pub fn draw(f: &mut Frame, app: &mut App) {
    let t = THEMES[app.theme];
    let full = f.area();
    f.render_widget(Block::default().style(base(&t)), full);

    if full.width < MIN_W || full.height < MIN_H {
        let msg = format!("terminal too small: need {MIN_W}x{MIN_H}, have {}x{}", full.width, full.height);
        f.render_widget(
            Paragraph::new(msg).style(fg(&t, t.yellow)).alignment(Alignment::Center),
            Rect::new(full.x, full.y + full.height / 2, full.width, 1),
        );
        return;
    }

    let w = full.width.min(MAX_W);
    let area = Rect::new(full.x + (full.width - w) / 2, full.y, w, full.height);

    app.hit.btns.clear();
    app.hit.speeds.clear();
    app.hit.reverbs.clear();
    app.hit.sources.clear();

    let outer = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(&t, t.cyan))
        .style(base(&t));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let lib_c = if app.lib_open { Constraint::Min(6) } else { Constraint::Length(2) };
    let fill_c = if app.lib_open { Constraint::Length(0) } else { Constraint::Min(0) };
    let rows = Layout::vertical([
        Constraint::Length(1), // title bar
        Constraint::Length(8), // art + info
        Constraint::Length(2), // progress + controls
        Constraint::Length(4), // audio
        Constraint::Length(3), // search
        lib_c,                 // library
        fill_c,                // filler when library is hidden
        Constraint::Length(1), // status
    ])
    .split(inner);

    draw_titlebar(f, app, &t, rows[0]);
    draw_top(f, app, &t, rows[1]);
    draw_progress(f, app, &t, rows[2]);
    draw_audio(f, app, &t, rows[3]);
    draw_search(f, app, &t, rows[4]);
    draw_library(f, app, &t, rows[5]);
    draw_status(f, app, &t, rows[7]);

    match app.mode {
        Mode::Settings => draw_settings(f, app, &t, full),
        Mode::Pick => draw_picker(f, app, &t, full),
        Mode::Name => draw_name(f, app, &t, full),
        Mode::Share => draw_share(f, app, &t, full),
        Mode::Join => draw_join(f, app, &t, full),
        Mode::Room => draw_room(f, app, &t, full),
        Mode::Lyrics => draw_lyrics(f, app, &t, full),
        _ => {}
    }
}

fn draw_titlebar(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let bar = Style::default().fg(t.dim).bg(t.panel);
    f.render_widget(Block::default().style(bar), r);

    let mut row = RowB::new(r.x + 1, r.y);
    row.put("● ", Style::default().fg(t.red).bg(t.panel));
    row.put("● ", Style::default().fg(t.yellow).bg(t.panel));
    row.put("● ", Style::default().fg(t.green).bg(t.panel));
    row.put(" ~/music - enigmatic player v1.8", bar);
    if let Some(room) = &app.room {
        let badge = if room.pending {
            " live: connecting ...".to_string()
        } else if room.host {
            format!(" live: {} host", room.code)
        } else {
            format!(" live: {} guest - following {}", room.code, room.host_name.clone())
        };
        let color = if room.pending { t.yellow } else if room.host { t.red } else { t.green };
        row.put(trunc(&badge, 28), Style::default().fg(color).bg(t.panel));
    }
    f.render_widget(Paragraph::new(row.line()), r);

    let label = "[ s: settings ] ";
    let lw = label.chars().count() as u16;
    let gx = r.x + r.width - lw;
    app.hit.gear = Rect::new(gx, r.y, lw, 1);
    f.render_widget(
        Paragraph::new(Span::styled(label, Style::default().fg(t.cyan).bg(t.panel))),
        app.hit.gear,
    );
}

fn draw_top(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let cols = Layout::horizontal([Constraint::Length(18), Constraint::Min(10)]).split(r);

    // album art
    let seed = app
        .now
        .as_ref()
        .map(|n| n.title.bytes().fold(7usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize)))
        .unwrap_or(3);
    let art_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.purple));
    let art_in = art_block.inner(cols[0]);
    f.render_widget(art_block, cols[0]);
    f.render_widget(Paragraph::new(art_lines(t, seed, art_in.width as usize)), art_in);

    // info
    let ir = Rect::new(cols[1].x + 2, cols[1].y, cols[1].width.saturating_sub(2), cols[1].height);
    let parts = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(ir);
    let iw = ir.width as usize;

    let origin = match &app.now {
        Some(n) if n.path.is_none() => "stream",
        _ => Provider::ALL[0].name(),
    };
    let mut np = format!("now playing - {}", origin);
    if app.buffering {
        np.push_str(" - buffering");
    }
    if (app.cfg.speed - 1.0).abs() > 0.001 {
        np.push_str(&format!(" - {:.2}x", app.cfg.speed));
    }
    if app.cfg.wet > 0 {
        np.push_str(&format!(" - reverb {}", REVERBS[app.cfg.reverb.min(4)].0));
    }
    f.render_widget(Paragraph::new(trunc(&np, iw)).style(fg(t, t.dim)), parts[0]);

    let (title, artist) = match &app.now {
        Some(n) => (n.title.clone(), n.artist.clone()),
        None => ("nothing playing".to_string(), "pick a track and press enter".to_string()),
    };
    f.render_widget(
        Paragraph::new(trunc(&title, iw)).style(fg(t, t.red).add_modifier(Modifier::BOLD)),
        parts[1],
    );
    f.render_widget(Paragraph::new(trunc(&artist, iw)).style(base(t)), parts[2]);

    // visualizer: 5 rows with eighth-block precision
    let glyphs = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let vh = parts[3].height as i32;
    let mut lines: Vec<Line<'static>> = Vec::new();
    for r_i in 0..vh {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for k in 0..BANDS {
            let v = app.bars.get(k).copied().unwrap_or(0.0);
            let h8 = (v * (vh as f32) * 8.0) as i32;
            let cell = (h8 - (vh - 1 - r_i) * 8).clamp(0, 8) as usize;
            let (g, col) = if cell == 0 && r_i == vh - 1 {
                ('▁', t.sel)
            } else {
                (glyphs[cell], bar_color(t, k))
            };
            spans.push(Span::styled(format!("{g}{g} "), fg(t, col)));
        }
        lines.push(Line::from(spans));
    }
    f.render_widget(Paragraph::new(lines).style(base(t)), parts[3]);
}

fn draw_progress(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let area = pad(r, 1);
    let bar = Rect::new(area.x, area.y, area.width, 1);
    app.hit.progress = bar;
    let w = bar.width as usize;
    let raw_filled: usize = if app.dur > 0.0 {
        ((app.pos / app.dur).clamp(0.0, 1.0) * w as f64).round() as usize
    } else {
        0
    };
    let filled = raw_filled.min(w);
    let line = Line::from(vec![
        Span::styled("█".repeat(filled), fg(t, t.red)),
        Span::styled("░".repeat(w - filled), fg(t, t.sel)),
    ]);
    f.render_widget(Paragraph::new(line), bar);

    let y = area.y + 1;
    f.render_widget(
        Paragraph::new(fmt_time(app.pos)).style(base(t)),
        Rect::new(area.x, y, 7, 1),
    );
    let dur_s = if app.dur > 0.0 { fmt_time(app.dur) } else { "--:--".to_string() };
    f.render_widget(
        Paragraph::new(dur_s).style(fg(t, t.dim)).alignment(Alignment::Right),
        Rect::new(area.x + area.width.saturating_sub(7), y, 7, 1),
    );

    // transport buttons, centered
    let play_label = if app.playing { "[||]" } else { "[> ]" };
    let items: [(&str, Btn); 5] = [
        ("[|<]", Btn::Prev),
        ("[-10]", Btn::Back),
        (play_label, Btn::Play),
        ("[+10]", Btn::Fwd),
        ("[>|]", Btn::Next),
    ];
    let total: u16 = items.iter().map(|(s, _)| s.chars().count() as u16).sum::<u16>() + 2 * 4;
    let start = area.x + area.width.saturating_sub(total) / 2;
    let mut row = RowB::new(start, y);
    for (i, (label, b)) in items.iter().enumerate() {
        if i > 0 {
            row.put("  ", base(t));
        }
        let rect = row.put(*label, fg(t, t.cyan));
        app.hit.btns.push((rect, *b));
    }
    f.render_widget(Paragraph::new(row.line()), Rect::new(start, y, total, 1));
}

fn draw_audio(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let blk = Block::default()
        .borders(Borders::TOP)
        .border_style(fg(t, t.line))
        .style(base(t));
    let inner = blk.inner(r);
    f.render_widget(blk, r);
    let x0 = inner.x + 1;
    let wdt = inner.width.saturating_sub(2);

    // volume
    {
        let y = inner.y;
        let mut row = RowB::new(x0, y);
        let muted = app.cfg.muted;
        app.hit.mute = row.put("vol    ", fg(t, if muted { t.red } else { t.green }));
        let on = if muted { 0 } else { (app.cfg.vol * NB_VOL as f32).round() as usize };
        let start = row.cur;
        for i in 0..NB_VOL {
            let col = if i < on {
                if i < 10 {
                    t.green
                } else if i < 14 {
                    t.yellow
                } else {
                    t.red
                }
            } else {
                t.sel
            };
            row.put("██ ", fg(t, col));
        }
        app.hit.vol = Rect::new(start, y, (NB_VOL * 3) as u16, 1);
        let pct = if muted { " mute".to_string() } else { format!(" {:>3}%", (app.cfg.vol * 100.0).round() as u32) };
        row.put(pct, base(t));
        f.render_widget(Paragraph::new(row.line()), Rect::new(x0, y, wdt, 1));
    }

    // speed
    {
        let y = inner.y + 1;
        let mut row = RowB::new(x0, y);
        row.put("speed ", fg(t, t.dim));
        for s in SPEEDS.iter() {
            let active = (app.cfg.speed - *s).abs() < 0.001;
            let st = if active {
                Style::default().fg(t.purple).bg(t.sel)
            } else {
                fg(t, t.text)
            };
            let rect = row.put(format!("{:.1} ", s), st);
            app.hit.speeds.push((rect, *s));
        }
        row.put(format!("{:.2}x", app.cfg.speed), fg(t, t.cyan));
        f.render_widget(Paragraph::new(row.line()), Rect::new(x0, y, wdt, 1));
    }

    // reverb
    {
        let y = inner.y + 2;
        let mut row = RowB::new(x0, y);
        row.put("reverb ", fg(t, t.dim));
        for (i, (name, _)) in REVERBS.iter().enumerate() {
            let active = app.cfg.reverb == i;
            let st = if active {
                Style::default().fg(t.purple).bg(t.sel)
            } else {
                fg(t, t.text)
            };
            let rect = row.put(format!(" {} ", name), st);
            app.hit.reverbs.push((rect, i));
            row.put(" ", base(t));
        }
        row.put(" ", base(t));
        let start = row.cur;
        let on = ((app.cfg.wet as f32 / 100.0) * NB_RV as f32).round() as usize;
        for i in 0..NB_RV {
            row.put("██ ", fg(t, if i < on { t.purple } else { t.sel }));
        }
        app.hit.rv_bar = Rect::new(start, y, (NB_RV * 3) as u16, 1);
        row.put(format!(" {:>3}%", app.cfg.wet), base(t));
        f.render_widget(Paragraph::new(row.line()), Rect::new(x0, y, wdt, 1));
    }
}

fn draw_search(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let blk = Block::default()
        .borders(Borders::TOP)
        .border_style(fg(t, t.line))
        .style(base(t));
    let inner = blk.inner(r);
    f.render_widget(blk, r);

    let provider = Provider::ALL[app.source];
    let mut row = RowB::new(inner.x + 1, inner.y);
    row.put(format!("> search {} ", provider.tag()), fg(t, t.cyan));
    if app.query.is_empty() && app.mode != Mode::Search {
        row.put("press / to search", fg(t, t.dim));
    } else {
        row.put(app.query.clone(), base(t));
        if app.mode == Mode::Search {
            row.put("█", fg(t, t.cyan));
        }
    }
    f.render_widget(
        Paragraph::new(row.line()),
        Rect::new(inner.x + 1, inner.y, inner.width.saturating_sub(2), 1),
    );

    let msg = if app.status.is_empty() {
        "keys: / search  enter play  a playlist  H host  U join  S share  s settings  q quit".to_string()
    } else {
        app.status.clone()
    };
    let w = inner.width.saturating_sub(2) as usize;
    let color = if app.searching || app.saving > 0 || app.buffering { t.yellow } else { t.dim };
    f.render_widget(
        Paragraph::new(trunc(&msg, w)).style(fg(t, color)),
        Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1),
    );
}

fn draw_library(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let blk = Block::default()
        .borders(Borders::TOP)
        .border_style(fg(t, t.line))
        .style(base(t));
    let inner = blk.inner(r);
    f.render_widget(blk, r);

    let header = Rect::new(inner.x, inner.y, inner.width, 1);
    app.hit.lib_header = header;
    let mut row = RowB::new(header.x + 1, header.y);
    row.put(
        format!("[{}] library", if app.lib_open { "-" } else { "+" }),
        fg(t, t.yellow),
    );
    let opened = if app.source == PLAYLISTS {
        app.open_list
            .and_then(|i| app.playlists.get(i))
            .map(|p| format!(" > {} (esc: back)", p.name))
    } else {
        None
    };
    row.put(
        opened.unwrap_or_else(|| " - sources and tracks (tab)".to_string()),
        fg(t, t.dim),
    );
    f.render_widget(Paragraph::new(row.line()), header);

    if !app.lib_open || inner.height < 2 {
        app.hit.tracks = Rect::default();
        return;
    }

    let body = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    let cols = Layout::horizontal([Constraint::Length(26), Constraint::Min(10)]).split(body);

    // sources
    let sblk = Block::default()
        .borders(Borders::RIGHT)
        .border_style(fg(t, t.sel))
        .style(base(t));
    let sin = sblk.inner(cols[0]);
    f.render_widget(sblk, cols[0]);
    for (i, p) in Provider::ALL.iter().enumerate() {
        if (i as u16) >= sin.height {
            break;
        }
        let y = sin.y + i as u16;
        let selected = i == app.source;
        let bg = if selected { t.sel } else { t.bg };
        let tag_col = match p {
            Provider::Local => t.green,
            Provider::YouTube => t.red,
            Provider::SoundCloud => t.yellow,
            Provider::Bandcamp => t.cyan,
            Provider::Playlists => t.purple,
        };
        let count = match p {
            Provider::Local => format!(" ({})", app.local.len()),
            Provider::Playlists => format!(" ({})", app.playlists.len()),
            _ => String::new(),
        };
        let rr = Rect::new(sin.x, y, sin.width, 1);
        let mut rb = RowB::new(rr.x, y);
        rb.put(if selected { "> " } else { "  " }, Style::default().fg(t.red).bg(bg));
        rb.put(format!("[{}]", p.tag()), Style::default().fg(tag_col).bg(bg));
        let name = trunc(&format!(" {}{}", p.name(), count), (sin.width as usize).saturating_sub(5));
        let used = rb.cur - rr.x;
        let name_w = name.chars().count() as u16;
        rb.put(name, Style::default().fg(t.text).bg(bg));
        let rest = sin.width.saturating_sub(used + name_w);
        rb.put(" ".repeat(rest as usize), Style::default().bg(bg));
        f.render_widget(Paragraph::new(rb.line()), rr);
        app.hit.sources.push((rr, i));
    }

    // tracks: only the visible rows are built, so huge libraries stay fast
    let area = cols[1];
    let hgt = area.height as usize;
    let total = app.rows();
    let in_overview = app.source == PLAYLISTS && app.open_list.is_none();

    if total == 0 {
        let provider = Provider::ALL[app.source];
        let msg = match provider {
            Provider::Local => {
                if app.query.is_empty() {
                    format!("no tracks in {}\nset the folder in settings (s)", app.cfg.dir)
                } else {
                    "no matches. try an artist name.".to_string()
                }
            }
            Provider::Playlists => {
                if in_overview {
                    "no playlists yet.\nselect a track and press a to start one,\nor press c here.".to_string()
                } else if app.query.is_empty() {
                    "this playlist is empty.\nselect a track in any source and press a.".to_string()
                } else {
                    "no matches.".to_string()
                }
            }
            _ if !app.have_ytdlp => "yt-dlp not found. install it to use this source.".to_string(),
            Provider::Bandcamp => "press / and paste a bandcamp track or album url".to_string(),
            _ => format!("press / to search {}, then enter", provider.name()),
        };
        f.render_widget(
            Paragraph::new(msg).style(fg(t, t.dim)),
            Rect::new(area.x + 1, area.y, area.width.saturating_sub(1), area.height),
        );
        app.hit.tracks = Rect::default();
        return;
    }

    if app.sel >= total {
        app.sel = total - 1;
    }
    if hgt > 0 {
        if app.sel < app.scroll {
            app.scroll = app.sel;
        } else if app.sel >= app.scroll + hgt {
            app.scroll = app.sel + 1 - hgt;
        }
    }
    if app.scroll >= total {
        app.scroll = total - 1;
    }

    let items: Vec<ListItem<'static>> = {
        let wd = area.width as usize;
        let end = (app.scroll + hgt).min(total);
        if in_overview {
            app.playlists[app.scroll..end]
                .iter()
                .enumerate()
                .map(|(i, pl)| {
                    let secs = pl.total_secs();
                    let right = if secs > 0.0 { fmt_time(secs) } else { "--:--".to_string() };
                    let n = pl.tracks.len();
                    let word = if n == 1 { "track" } else { "tracks" };
                    let avail = wd.saturating_sub(right.chars().count() + 2);
                    let left = trunc(
                        &format!("  {:02}  {}  ({} {})", app.scroll + i + 1, pl.name, n, word),
                        avail,
                    );
                    let padn = avail.saturating_sub(left.chars().count());
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{}{} ", left, " ".repeat(padn)), fg(t, t.text)),
                        Span::styled(right, fg(t, t.dim)),
                    ]))
                })
                .collect()
        } else {
            let tracks = app.list();
            let now = app.now.as_ref();
            tracks[app.scroll..end]
                .iter()
                .enumerate()
                .map(|(i, tr)| {
                    let playing = now.map(|n| same(n, tr)).unwrap_or(false);
                    let marker = if playing { ">" } else { " " };
                    let dur = if tr.secs > 0.0 { fmt_time(tr.secs) } else { "--:--".to_string() };
                    let avail = wd.saturating_sub(dur.chars().count() + 2);
                    let left = trunc(
                        &format!("{} {:02}  {} - {}", marker, app.scroll + i + 1, tr.title, tr.artist),
                        avail,
                    );
                    let padn = avail.saturating_sub(left.chars().count());
                    let st = if playing { fg(t, t.red) } else { fg(t, t.text) };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("{}{} ", left, " ".repeat(padn)), st),
                        Span::styled(dur, fg(t, t.dim)),
                    ]))
                })
                .collect()
        }
    };

    let mut st = ListState::default().with_selected(Some(app.sel - app.scroll));
    let list = List::new(items).highlight_style(Style::default().bg(t.sel).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list, area, &mut st);
    app.hit.tracks = area;
    app.hit.tracks_offset = app.scroll;
}

fn draw_status(f: &mut Frame, app: &mut App, t: &Theme, r: Rect) {
    let st = Style::default().bg(t.panel);
    f.render_widget(Block::default().style(st), r);
    let vol = if app.cfg.muted {
        "vol muted".to_string()
    } else {
        format!("vol {}%", (app.cfg.vol * 100.0).round() as u32)
    };
    let fx = if app.cfg.wet > 0 {
        format!("reverb {} {}%", REVERBS[app.cfg.reverb.min(4)].0, app.cfg.wet)
    } else {
        "reverb off".to_string()
    };
    let line = Line::from(vec![
        Span::styled(format!(" {}", vol), st.fg(t.green)),
        Span::styled(format!("   speed {:.2}x - {}", app.cfg.speed, fx), st.fg(t.purple)),
        Span::styled(format!("   theme: {}", THEMES[app.theme].name), st.fg(t.dim)),
    ]);
    f.render_widget(Paragraph::new(line), r);
}

fn source_label(source: &str) -> &'static str {
    match source {
        "file" => "from a .lrc/.txt file next to the song",
        "tags" => "from embedded audio tags",
        "cache" => "from the saved lyrics cache",
        "lrclib" => "from lrclib.net",
        _ => "lyrics",
    }
}

fn draw_lyrics(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let pop = centered(area, 84, 28);
    f.render_widget(Clear, pop);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.yellow))
        .title(Span::styled(" lyrics ", fg(t, t.yellow)))
        .style(base(t));
    let inner = pad(blk.inner(pop), 1);
    f.render_widget(blk, pop);

    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(inner);
    let w = rows[0].width as usize;

    let track = app
        .now
        .as_ref()
        .map(|n| format!("{} - {}", n.title, n.artist))
        .unwrap_or_else(|| "nothing playing".to_string());
    let title = Line::from(Span::styled(
        trunc(&track, w),
        base(t).add_modifier(Modifier::BOLD),
    ));

    match &app.lyrics {
        lyrics::State::Idle | lyrics::State::Loading => {
            f.render_widget(
                Paragraph::new(vec![
                    title,
                    Line::from(Span::styled("looking for lyrics ...", fg(t, t.dim))),
                ]),
                rows[0],
            );
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "searching the song file, embedded tags, the cache, then lrclib.net",
                    fg(t, t.dim),
                ))),
                rows[1],
            );
        }
        lyrics::State::Missing(message) => {
            f.render_widget(
                Paragraph::new(vec![
                    title,
                    Line::from(Span::styled("no lyrics available", fg(t, t.red))),
                ]),
                rows[0],
            );
            let mut lines = vec![
                Line::from(Span::styled(trunc(message, w), fg(t, t.dim))),
                Line::from(""),
            ];
            if !lyrics::online_enabled() {
                lines.push(Line::from(Span::styled(
                    "online lookup is off: put a .lrc or .txt file next to the song,",
                    fg(t, t.dim),
                )));
                lines.push(Line::from(Span::styled(
                    "or embed lyrics in its tags.",
                    fg(t, t.dim),
                )));
            }
            f.render_widget(Paragraph::new(lines), rows[1]);
        }
        lyrics::State::Ready(lyric) => {
            let follow = if app.lyrics_follow {
                "following the song"
            } else {
                "manual scroll"
            };
            f.render_widget(
                Paragraph::new(vec![
                    title,
                    Line::from(Span::styled(
                        trunc(&format!("{} · {}", source_label(lyric.source), follow), w),
                        fg(t, t.dim),
                    )),
                ]),
                rows[0],
            );

            if lyric.lines.is_empty() {
                let msg = if lyric.instrumental {
                    "this track is instrumental - no lyrics to show"
                } else {
                    "the lyrics for this track are empty"
                };
                f.render_widget(
                    Paragraph::new(Line::from(Span::styled(msg, fg(t, t.dim)))),
                    rows[1],
                );
            } else {
                let current = lyric.current(app.pos);
                let body_h = rows[1].height.max(1) as usize;
                let top = if app.lyrics_follow {
                    current.map(|i| i.saturating_sub(body_h / 2)).unwrap_or(0)
                } else {
                    app.lyrics_scroll
                };
                let max_top = lyric.lines.len().saturating_sub(body_h);
                let top = top.min(max_top);
                app.lyrics_scroll = top;

                let cur_index = current.unwrap_or(usize::MAX);
                let mut lines: Vec<Line<'static>> = Vec::new();
                for (i, line) in lyric.lines.iter().enumerate().skip(top).take(body_h) {
                    let is_current = current == Some(i);
                    let text_style = if is_current {
                        Style::default()
                            .fg(t.yellow)
                            .bg(t.sel)
                            .add_modifier(Modifier::BOLD)
                    } else if lyric.synced() && i < cur_index {
                        fg(t, t.dim)
                    } else {
                        fg(t, t.text)
                    };
                    let mut spans = vec![Span::styled(
                        if is_current { "▶ " } else { "  " },
                        fg(t, if is_current { t.yellow } else { t.dim }),
                    )];
                    if let Some(at) = line.at {
                        spans.push(Span::styled(
                            format!("{:>4}  ", fmt_time(at)),
                            fg(t, t.dim),
                        ));
                    }
                    spans.push(Span::styled(trunc(&line.text, w.saturating_sub(8)), text_style));
                    lines.push(Line::from(spans));
                }
                f.render_widget(Paragraph::new(lines), rows[1]);
            }
        }
    }

    let hint = match &app.lyrics {
        lyrics::State::Ready(l) if l.synced() => {
            "up/down: scroll   f: follow   r: reload   y/esc: close"
        }
        lyrics::State::Ready(_) => "plain lyrics (no timing)   r: reload   y/esc: close",
        _ => "r: try again   y/esc: close",
    };
    let footer = vec![
        Line::from(Span::styled(
            if app.now.is_some() && app.playing {
                format!("{}   (space: play/pause)", hint)
            } else {
                hint.to_string()
            },
            fg(t, t.cyan),
        )),
        Line::from(Span::styled(
            format!("playing at {} / {}", fmt_time(app.pos), fmt_time(app.dur)),
            fg(t, t.dim),
        )),
    ];
    f.render_widget(Paragraph::new(footer), rows[2]);
}

fn draw_settings(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let w = 72u16.min(area.width.saturating_sub(2));
    let h = 28u16.min(area.height.saturating_sub(2));
    let pop = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h);
    f.render_widget(Clear, pop);

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.purple))
        .title(Span::styled(" settings ", fg(t, t.yellow)))
        .style(base(t));
    let inner = blk.inner(pop);
    f.render_widget(blk, pop);

    let cols = Layout::horizontal([Constraint::Length(16), Constraint::Min(10)]).split(inner);

    // section list
    let nav_blk = Block::default()
        .borders(Borders::RIGHT)
        .border_style(fg(t, t.sel))
        .style(base(t));
    let nav_in = nav_blk.inner(cols[0]);
    f.render_widget(nav_blk, cols[0]);
    let names = ["themes", "local music", "audio output", "about"];
    let nav: Vec<Line<'static>> = names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if i == app.set_tab {
                Line::from(Span::styled(
                    format!("> {:<12}", n),
                    Style::default().fg(t.red).bg(t.sel),
                ))
            } else {
                Line::from(Span::styled(format!("  {}", n), fg(t, t.text)))
            }
        })
        .collect();
    f.render_widget(Paragraph::new(nav), nav_in);

    let content = pad(cols[1], 1);
    match app.set_tab {
        0 => {
            let parts = Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).split(content);
            f.render_widget(
                Paragraph::new("up/down: preview a theme\nenter: keep it   tab: next section").style(fg(t, t.dim)),
                parts[0],
            );
            let items: Vec<ListItem<'static>> = THEMES
                .iter()
                .map(|th| {
                    let sw = |c: Color| Span::styled("██", Style::default().fg(c).bg(th.bg));
                    ListItem::new(Line::from(vec![
                        sw(th.red),
                        Span::styled(" ", Style::default().bg(th.bg)),
                        sw(th.cyan),
                        Span::styled(" ", Style::default().bg(th.bg)),
                        sw(th.green),
                        Span::styled(" ", Style::default().bg(th.bg)),
                        sw(th.yellow),
                        Span::styled(format!("  {}", th.name), fg(t, t.text)),
                    ]))
                })
                .collect();
            let list = List::new(items).highlight_style(Style::default().bg(t.sel).add_modifier(Modifier::BOLD));
            f.render_stateful_widget(list, parts[1], &mut app.theme_state);
        }
        1 => {
            let mut lines: Vec<Line<'static>> = vec![
                Line::from(Span::styled("local music folder", fg(t, t.yellow))),
                Line::from(Span::styled(
                    "Downloads are saved here, and the",
                    fg(t, t.dim),
                )),
                Line::from(Span::styled(
                    "local files source reads from it.",
                    fg(t, t.dim),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("> ", fg(t, t.cyan)),
                    Span::styled(app.dir_input.clone(), base(t)),
                    Span::styled("█", fg(t, t.cyan)),
                ]),
            ];
            match &app.dir_msg {
                Some((m, ok)) => lines.push(Line::from(Span::styled(
                    m.clone(),
                    fg(t, if *ok { t.green } else { t.red }),
                ))),
                None => lines.push(Line::from("")),
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled("up/down: quick picks   enter: save", fg(t, t.dim))));
            for p in crate::app::PICKS.iter() {
                lines.push(Line::from(Span::styled(format!("  {}", p), fg(t, t.text))));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled("current: ", fg(t, t.dim)),
                Span::styled(app.cfg.dir.clone(), fg(t, t.cyan)),
            ]));
            f.render_widget(Paragraph::new(lines), content);
        }
        2 => {
            let parts = Layout::vertical([Constraint::Length(4), Constraint::Min(3)]).split(content);
            let now = if app.cfg.device.is_empty() { "system default" } else { app.cfg.device.as_str() };
            let head = format!(
                "choose where the sound plays\nup/down: pick   enter: use it\n* marks the active output\nnow: {}",
                trunc(now, (content.width as usize).saturating_sub(6))
            );
            f.render_widget(Paragraph::new(head).style(fg(t, t.dim)), parts[0]);
            let w = parts[1].width.saturating_sub(3) as usize;
            let items: Vec<ListItem<'static>> = app
                .devices
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let active = (i == 0 && app.cfg.device.is_empty()) || *d == app.cfg.device;
                    ListItem::new(Line::from(vec![
                        Span::styled(if active { "* " } else { "  " }, fg(t, t.green)),
                        Span::styled(trunc(d, w), fg(t, t.text)),
                    ]))
                })
                .collect();
            let list = List::new(items).highlight_style(Style::default().bg(t.sel).add_modifier(Modifier::BOLD));
            f.render_stateful_widget(list, parts[1], &mut app.dev_state);
        }
        // 3 and beyond: the about page (set_tab is clamped to 0..3 by the key handler)
        _ => {
            let key = |k: &str, d: &str| -> Line<'static> {
                Line::from(vec![
                    Span::styled(format!("{:<14}", k), fg(t, t.text)),
                    Span::styled(d.to_string(), fg(t, t.dim)),
                ])
            };
            let lines: Vec<Line<'static>> = vec![
                Line::from(Span::styled("about enigmatic player", fg(t, t.yellow))),
                Line::from(Span::styled("A pixel-style terminal music player.", base(t))),
                Line::from(""),
                Line::from(vec![
                    Span::styled("version   ", fg(t, t.dim)),
                    Span::styled("v1.8", base(t)),
                ]),
                Line::from(vec![
                    Span::styled("made by   ", fg(t, t.dim)),
                    Span::styled("Jamal Tiliouan", fg(t, t.cyan)),
                ]),
                Line::from(vec![
                    Span::styled("project   ", fg(t, t.dim)),
                    Span::styled("open source", fg(t, t.green)),
                ]),
                Line::from(""),
                key("space", "play or pause"),
                key("left / right", "seek 5 seconds"),
                key("up / down", "move in the list"),
                key("enter", "play (online tracks stream)"),
                key("d", "save online track as mp3"),
                key("/", "search"),
                key("tab", "hide or show library"),
                key("1-5", "choose source (5 = playlists)"),
                key("a / A", "add selected / playing track to a playlist"),
                key("c R x", "playlists: new, rename, delete"),
                key("J / K", "move track or playlist down / up"),
                key("S", "share the open playlist as a code"),
                key("H", "host a listen-together room / room popup"),
                key("U", "join a room or import a playlist code"),
                key("L", "leave the shared room"),
                key("- / =", "volume"),
                key("m", "mute"),
                key("[ / ]", "slower and faster"),
                key("r  < >", "reverb preset and amount"),
                key("n / p", "next and previous"),
                key("y", "lyrics for the current track"),
                key("q", "quit"),
            ];
            f.render_widget(Paragraph::new(lines), content);
        }
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 2, w, h)
}

/// "Add to playlist": choose an existing playlist, or make a new one.
fn draw_picker(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let pop = centered(area, 54, (app.playlists.len() as u16 + 6).clamp(8, 20));
    f.render_widget(Clear, pop);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.purple))
        .title(Span::styled(" add to playlist ", fg(t, t.yellow)))
        .style(base(t));
    let inner = blk.inner(pop);
    f.render_widget(blk, pop);

    let parts = Layout::vertical([Constraint::Length(3), Constraint::Min(2)]).split(pad(inner, 1));
    let title = app.pending.as_ref().map(|p| p.title.clone()).unwrap_or_default();
    let head = format!(
        "adding: {}\nup/down: choose   enter: add\nesc: cancel",
        trunc(&title, (parts[0].width as usize).saturating_sub(9))
    );
    f.render_widget(Paragraph::new(head).style(fg(t, t.dim)), parts[0]);

    let mut items: Vec<ListItem<'static>> = vec![ListItem::new(Line::from(Span::styled(
        "+ new playlist",
        fg(t, t.green),
    )))];
    for p in app.playlists.iter() {
        let n = p.tracks.len();
        let word = if n == 1 { "track" } else { "tracks" };
        items.push(ListItem::new(Line::from(Span::styled(
            trunc(&format!("{}  ({} {})", p.name, n, word), (parts[1].width as usize).saturating_sub(1)),
            fg(t, t.text),
        ))));
    }
    let list = List::new(items).highlight_style(Style::default().bg(t.sel).add_modifier(Modifier::BOLD));
    f.render_stateful_widget(list, parts[1], &mut app.pick_state);
}

/// Name box for a new or renamed playlist.
fn draw_name(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let pop = centered(area, 50, 8);
    f.render_widget(Clear, pop);
    let title = if app.rename.is_some() { " rename playlist " } else { " new playlist " };
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.purple))
        .title(Span::styled(title, fg(t, t.yellow)))
        .style(base(t));
    let inner = pad(blk.inner(pop), 1);
    f.render_widget(blk, pop);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled("name:", fg(t, t.dim))),
        Line::from(vec![
            Span::styled("> ", fg(t, t.cyan)),
            Span::styled(app.name_input.clone(), base(t)),
            Span::styled("█", fg(t, t.cyan)),
        ]),
    ];
    match &app.name_err {
        Some(e) => lines.push(Line::from(Span::styled(e.clone(), fg(t, t.red)))),
        None => lines.push(Line::from("")),
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("enter: save   esc: cancel", fg(t, t.dim))));
    f.render_widget(Paragraph::new(lines), inner);
}

/// "Share playlist": the share code, where the code file went, and copy.
fn draw_share(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let pop = centered(area, 76, 10);
    f.render_widget(Clear, pop);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.green))
        .title(Span::styled(" share playlist ", fg(t, t.yellow)))
        .style(base(t));
    let inner = pad(blk.inner(pop), 1);
    f.render_widget(blk, pop);

    let w = inner.width as usize;
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled("playing together needs the same files, so this code carries ", fg(t, t.dim)),
            Span::styled("metadata + online urls only", fg(t, t.text)),
            Span::styled(", never file paths.", fg(t, t.dim)),
        ]),
        Line::from(vec![
            Span::styled("playlist: ", fg(t, t.dim)),
            Span::styled(trunc(&app.share_title, w.saturating_sub(11)), fg(t, t.cyan)),
        ]),
        Line::from(""),
        Line::from(Span::styled("share code:", fg(t, t.dim))),
        Line::from(Span::styled(
            tail_trunc(&app.share_code, w),
            fg(t, t.green),
        )),
    ];
    match &app.share_path {
        Some(p) => lines.push(Line::from(vec![
            Span::styled("saved to: ", fg(t, t.dim)),
            Span::styled(trunc(&p.to_string_lossy(), w.saturating_sub(10)), fg(t, t.text)),
        ])),
        None => lines.push(Line::from("")),
    }
    lines.push(Line::from(""));
    let hint = if app.share_copied { "code has been copied - paste it anywhere" } else { "enter: copy code (OSC52)   esc: close" };
    lines.push(Line::from(Span::styled(hint, fg(t, t.cyan))));
    f.render_widget(Paragraph::new(lines), inner);
}

/// "Join or import": paste a room invite or a playlist share code. Rooms
/// heard on this network (someone holding H nearby) appear as a list.
fn draw_join(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let ads = app.room_ads.len() as u16;
    let pop = centered(area, 70, 9 + ads.min(6));
    f.render_widget(Clear, pop);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.purple))
        .title(Span::styled(" join or import ", fg(t, t.yellow)))
        .style(base(t));
    let inner = pad(blk.inner(pop), 1);
    f.render_widget(blk, pop);

    let w = inner.width.saturating_sub(3) as usize;
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "paste a room invite (EROOM2:...), a room code, or a playlist share code.",
            fg(t, t.dim),
        )),
        Line::from(Span::styled(
            "a path to an .elist.txt file works too.",
            fg(t, t.dim),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", fg(t, t.cyan)),
            Span::styled(tail_trunc(&app.join_input, w), base(t)),
            Span::styled("█", fg(t, t.cyan)),
        ]),
    ];
    match &app.join_err {
        Some(e) => lines.push(Line::from(Span::styled(e.clone(), fg(t, t.red)))),
        None => lines.push(Line::from("")),
    }
    if ads > 0 {
        lines.push(Line::from(Span::styled(
            "rooms on this network:",
            fg(t, t.cyan),
        )));
        for (i, ad) in app.room_ads.iter().enumerate() {
            if (i as u16) >= 6 {
                break;
            }
            let sel = i == app.room_pick;
            let mark = if sel { "> " } else { "  " };
            lines.push(Line::from(vec![
                Span::styled(mark, fg(t, if sel { t.cyan } else { t.dim })),
                Span::styled(
                    trunc(&format!("{} — code {}", ad.host_name, ad.code), w),
                    fg(t, if sel { t.text } else { t.sel }),
                ),
            ]));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        if ads > 0 {
            "up/down: pick a room   enter: join (or paste first)   esc: cancel"
        } else {
            "enter: connect   esc: cancel"
        },
        fg(t, t.dim),
    )));
    f.render_widget(Paragraph::new(lines), inner);
}

/// "Listening together": who is here, what is playing, copy invite, leave.
fn draw_room(f: &mut Frame, app: &mut App, t: &Theme, area: Rect) {
    let mut names = String::new();
    let mut invite = String::new();
    let mut role = String::new();
    let mut label = String::new();
    let mut server = String::new();
    let mut code = String::new();
    let mut pending = false;
    if let Some(r) = &app.room {
        names = r.members.join(", ");
        role = if r.host { "host".into() } else { "guest".into() };
        invite = r.invite();
        label = r.me.clone();
        server = r.server.clone();
        code = r.code.clone();
        pending = r.pending;
    }
    let pop = centered(area, 76, 14);
    f.render_widget(Clear, pop);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .border_style(fg(t, t.green))
        .title(Span::styled(" listening together ", fg(t, t.yellow)))
        .style(base(t));
    let inner = pad(blk.inner(pop), 1);
    f.render_widget(blk, pop);

    let w = inner.width as usize;
    let now = app.now.as_ref().map(|n| format!("{} - {}", n.title, n.artist)).unwrap_or_default();
    let head = if pending {
        "connecting ...".to_string()
    } else {
        format!("room {} ({} listeners live here)", code, names.split(", ").filter(|s| !s.is_empty()).count())
    };
    // Honest note about how this room is reachable.
    let reach = if pending {
        "opening an encrypted peer-to-peer connection ...".to_string()
    } else if role == "host" {
        "reach: friends anywhere - direct when possible, encrypted relay otherwise".to_string()
    } else {
        format!("connected to host {}", tail_trunc(&server, w.saturating_sub(18)))
    };
    let reach_ok = !pending;
    let lines = vec![
        Line::from(Span::styled(trunc(&head, w), fg(t, t.green))),
        Line::from(Span::styled(trunc(&reach, w), fg(t, if reach_ok { t.green } else { t.dim }))),
        Line::from(Span::styled(
            trunc(&format!("you are the {}, name: {}", role, label), w),
            fg(t, t.dim),
        )),
        Line::from(Span::styled(
            trunc(&format!("now playing: {}", now,), w),
            fg(t, t.text),
        )),
        Line::from(Span::styled(
            trunc(&format!("invite: {}", tail_trunc(&invite, w.saturating_sub(9))), w),
            fg(t, t.cyan),
        )),
        Line::from(""),
        Line::from(Span::styled(
            trunc(&format!("listening: {} people", names.split(", ").filter(|s| !s.is_empty()).count()), w),
            fg(t, t.dim),
        )),
        Line::from(Span::styled(trunc(&names, w), fg(t, t.text))),
        Line::from(""),
        Line::from(Span::styled(
            "each listener plays their own copy (local file or online url).",
            fg(t, t.dim),
        )),
        Line::from(Span::styled(
            if app.following() {
                "you follow the host; only they change tracks and seek."
            } else {
                "you are the host; your play/pause and seek sync everyone."
            },
            fg(t, t.dim),
        )),
        Line::from(""),
        Line::from(Span::styled("enter/c: copy invite   L: leave   esc: close", fg(t, t.cyan))),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}
