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
use std::path::PathBuf;

use tuigps_rs::gps_logger::LogMode;
use tuigps_rs::{app, config};

#[derive(Parser)]
#[command(name = "tuigps-rs", about = "Terminal GPS monitor")]
struct Cli {
    /// gpsd host; overrides the config file [default: localhost]
    #[arg(long)]
    host: Option<String>,
    /// gpsd port; overrides the config file [default: 2947]
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    port: Option<u16>,
    /// Config file [default: $XDG_CONFIG_HOME/tuigps-rs/config.toml or
    /// ~/.config/tuigps-rs/config.toml]
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    /// Log file format for `l`; overrides the config file [default: gpx]
    #[arg(long, value_name = "gpx|csv|both", value_parser = clap::value_parser!(LogMode))]
    log_format: Option<LogMode>,
    /// Directory for log files; overrides the config file [default: .]
    #[arg(long, value_name = "DIR")]
    log_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config_path = cli.config.or_else(config::default_path);
    let mut startup = config::startup(config_path, cli.host, cli.port);
    config::apply_log_overrides(&mut startup.config, cli.log_format, cli.log_dir);

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

    let result = app::run(&mut terminal, startup).await;

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
