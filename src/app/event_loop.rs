//! The async event loop: terminal input, gpsd events, command output,
//! the 1 Hz tick and redraw throttling.

use anyhow::Result;
use futures::StreamExt;
use ratatui::prelude::*;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

use super::App;
use crate::config::Startup;
use crate::gpsd_client::{self, TaggedEvent};
use crate::ui;

/// Minimum interval between redraws triggered by gpsd/command traffic (~20 fps).
const FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// Upper bound on gpsd events drained per loop iteration before drawing.
const MAX_DRAIN: usize = 256;

/// Consecutive terminal input errors tolerated before giving up.
const MAX_INPUT_ERRORS: u32 = 20;

/// Redraw scheduling: urgent redraws (keys, tick) happen immediately; data
/// redraws are coalesced to at most one per [`FRAME_INTERVAL`].
#[derive(Debug)]
struct FrameLimiter {
    dirty: bool,
    urgent: bool,
    last_draw: Option<Instant>,
}

impl FrameLimiter {
    fn new() -> Self {
        Self {
            dirty: true,
            urgent: true,
            last_draw: None,
        }
    }

    fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    fn mark_urgent(&mut self) {
        self.dirty = true;
        self.urgent = true;
    }

    fn should_draw(&self, now: Instant) -> bool {
        self.dirty
            && (self.urgent
                || self
                    .last_draw
                    .is_none_or(|t| now.duration_since(t) >= FRAME_INTERVAL))
    }

    fn drew(&mut self, now: Instant) {
        self.dirty = false;
        self.urgent = false;
        self.last_draw = Some(now);
    }

    /// When the next coalesced redraw is due, if one is pending.
    fn deadline(&self) -> Option<Instant> {
        if !self.dirty {
            return None;
        }
        Some(
            self.last_draw
                .map_or_else(Instant::now, |t| t + FRAME_INTERVAL),
        )
    }
}

/// Run the TUI until quit. `startup` comes from [`crate::config::startup`]
/// (config file merged with CLI overrides).
pub async fn run(terminal: &mut Terminal<impl Backend>, startup: Startup) -> Result<()> {
    let mut app = App::from_startup(startup);
    let result = event_loop(terminal, &mut app).await;
    // Always close the log so the GPX file is valid, even on error.
    app.stop_logging();
    result
}

async fn event_loop(terminal: &mut Terminal<impl Backend>, app: &mut App) -> Result<()> {
    // Channel for gpsd events, tagged with the connection generation
    let (tx, mut rx) = mpsc::channel::<TaggedEvent>(100);
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    let spawn_gpsd = |app: &App| {
        tokio::spawn(gpsd_client::gpsd_task(
            app.host.clone(),
            app.port,
            app.connection_generation,
            tx.clone(),
            shutdown_rx.clone(),
        ))
    };
    let mut gpsd_handle = spawn_gpsd(app);

    // Event stream
    let mut event_reader = crossterm::event::EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut frames = FrameLimiter::new();
    let mut input_errors = 0u32;

    let result = loop {
        let now = Instant::now();
        if frames.should_draw(now) {
            if let Err(e) = terminal.draw(|f| ui::render(f, app)) {
                break Err(e.into());
            }
            frames.drew(now);
        }

        let deadline = frames.deadline();
        tokio::select! {
            biased;
            event = event_reader.next() => match event {
                Some(Ok(event)) => {
                    input_errors = 0;
                    app.handle_event(event);
                    frames.mark_urgent();
                }
                Some(Err(e)) => {
                    input_errors += 1;
                    if input_errors >= MAX_INPUT_ERRORS {
                        // Input is unusable; quit rather than spin unquittable.
                        break Err(anyhow::anyhow!("terminal input failed: {}", e));
                    }
                    app.set_status(format!("Terminal input error: {}", e));
                    frames.mark_urgent();
                }
                None => {
                    // Input stream ended: no way to receive 'q', so quit.
                    app.should_quit = true;
                }
            },
            _ = tick.tick() => {
                app.tick();
                frames.mark_urgent();
            }
            Some(cmd_output) = app.cmd_rx.recv() => {
                // Split multi-line output into separate log entries
                for line in cmd_output.lines() {
                    if !line.trim().is_empty() {
                        app.device_config.push_log(line);
                    }
                }
                frames.mark_dirty();
            }
            Some(status) = app.status_rx.recv() => {
                app.set_status(status);
                frames.mark_urgent();
            }
            Some(event) = rx.recv() => {
                let mut changed = app.handle_tagged_event(event);
                // Drain whatever else is queued so one frame covers it all.
                for _ in 0..MAX_DRAIN {
                    match rx.try_recv() {
                        Ok(event) => changed |= app.handle_tagged_event(event),
                        Err(_) => break,
                    }
                }
                if changed {
                    frames.mark_dirty();
                }
            }
            _ = sleep_until_opt(deadline), if deadline.is_some() => {}
        }

        app.dispatch_device_commands();
        app.dispatch_clock_sync();

        // Handle reconnect
        if app.reconnect_requested {
            app.reconnect_requested = false;
            // Abort old task; any events it already queued carry the old
            // generation and are discarded by handle_tagged_event.
            gpsd_handle.abort();
            let _ = gpsd_handle.await;
            app.begin_reconnect();
            gpsd_handle = spawn_gpsd(app);
            frames.mark_urgent();
        }

        if app.should_quit {
            break Ok(());
        }
    };

    let _ = shutdown_tx.send(true);
    gpsd_handle.abort();
    result
}

async fn sleep_until_opt(deadline: Option<Instant>) {
    if let Some(d) = deadline {
        tokio::time::sleep_until(d.into()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Frame limiter ===

    #[test]
    fn test_frame_limiter_coalesces_data_redraws() {
        let mut f = FrameLimiter::new();
        let t0 = Instant::now();
        assert!(f.should_draw(t0)); // first frame
        f.drew(t0);
        assert!(!f.should_draw(t0));
        assert!(f.deadline().is_none());

        f.mark_dirty();
        assert!(!f.should_draw(t0 + Duration::from_millis(10)));
        assert_eq!(f.deadline(), Some(t0 + FRAME_INTERVAL));
        assert!(f.should_draw(t0 + FRAME_INTERVAL));
    }

    #[test]
    fn test_frame_limiter_urgent_draws_immediately() {
        let mut f = FrameLimiter::new();
        let t0 = Instant::now();
        f.drew(t0);
        f.mark_urgent();
        assert!(f.should_draw(t0 + Duration::from_millis(1)));
    }
}
