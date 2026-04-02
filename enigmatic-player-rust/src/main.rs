mod app;
mod audio;
mod beacon;
mod config;
mod library;
mod punch;
mod playlists;
mod server;
mod share;
mod stream;
mod themes;
mod together;
mod ui;

use std::io;
use std::time::{Duration, Instant};

use ratatui::{
    backend::CrosstermBackend,
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind, MouseEventKind},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    Terminal,
};

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

/// Sync-server mode: `enigmatic-player --serve [host:port]`.
/// This binary is either the player or the relay, never both.
fn serve_mode(args: &[String]) -> Option<io::Result<()>> {
    let flag = args.iter().position(|a| a == "--serve" || a == "--server")?;
    let addr = args
        .get(flag + 1)
        .cloned()
        .unwrap_or_else(|| "0.0.0.0:8080".to_string());
    Some(server::run(&addr))
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: enigmatic-player [--serve [host:port]]");
        println!("  without flags: the terminal player");
        println!("  --serve 0.0.0.0:8080: run the listen-together sync relay");
        return Ok(());
    }
    if let Some(r) = serve_mode(&args) {
        return r;
    }

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut app = app::App::new();
    let result = run(&mut terminal, &mut app);

    app.shutdown();
    restore_terminal();
    let _ = terminal.show_cursor();
    result
}

/// Draws only when something changed, and sleeps longer while idle,
/// so the player uses almost no CPU when paused.
fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut app::App) -> io::Result<()> {
    let mut last = Instant::now();
    let mut dirty = true;

    loop {
        if app.needs_clear {
            terminal.clear()?;
            app.needs_clear = false;
            dirty = true;
        }
        if dirty {
            terminal.draw(|f| ui::draw(f, app))?;
            dirty = false;
        }

        let tick = if app.playing {
            Duration::from_millis(50)
        } else {
            Duration::from_millis(250)
        };
        let timeout = tick.saturating_sub(last.elapsed());

        if event::poll(timeout)? {
            loop {
                match event::read()? {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        app.on_key(k);
                        dirty = true;
                    }
                    Event::Mouse(m) => {
                        // plain mouse movement is ignored: no redraw storms
                        if !matches!(m.kind, MouseEventKind::Moved) {
                            app.on_mouse(m);
                            dirty = true;
                        }
                    }
                    Event::Resize(_, _) => dirty = true,
                    _ => {}
                }
                if !event::poll(Duration::from_millis(0))? {
                    break;
                }
            }
        }

        if last.elapsed() >= tick {
            let dt = last.elapsed().as_secs_f64();
            last = Instant::now();
            if app.tick(dt) {
                dirty = true;
            }
        }

        if app.quit {
            break;
        }
    }
    Ok(())
}
