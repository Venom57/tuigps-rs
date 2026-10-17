use anyhow::Result;
use clap::Parser;
use crossterm::{
    cursor::Show,
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::prelude::*;
use std::io::{self, Stdout};
use std::panic;

use tuigps_rs::app;

#[derive(Parser)]
#[command(name = "tuigps-rs", about = "Terminal GPS monitor")]
struct Cli {
    #[arg(long, default_value = "localhost")]
    host: String,
    #[arg(long, default_value_t = 2947)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    install_panic_hook();

    // If any setup step fails, undo whatever already succeeded before
    // reporting the error so the user's shell is not left in raw mode.
    let mut terminal = match setup_terminal() {
        Ok(t) => t,
        Err(e) => {
            restore_terminal();
            return Err(e);
        }
    };

    let result = app::run(&mut terminal, &cli.host, cli.port).await;

    restore_terminal();
    result
}

/// Enter raw mode + alternate screen. Mouse capture is intentionally not
/// enabled: nothing handles mouse events and capturing them blocks the
/// terminal's native text selection (e.g. copying coordinates).
fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    Ok(terminal)
}

/// Best-effort terminal restore; safe to call more than once and from a
/// panic hook.
fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

/// Restore the terminal before the panic message is printed. A panic on a
/// non-main thread (e.g. a tokio worker running the gpsd task) would
/// otherwise leave the UI loop drawing onto a restored, cooked-mode screen,
/// so exit the process in that case.
fn install_panic_hook() {
    let original_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        restore_terminal();
        original_hook(info);
        if std::thread::current().name() != Some("main") {
            std::process::exit(101);
        }
    }));
}
