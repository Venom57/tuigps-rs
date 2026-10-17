//! Settings dialog: field navigation, text editing and apply/cancel.

use crossterm::event::{KeyCode, KeyModifiers};

use super::App;
use crate::config;

// Settings overlay field indices
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SettingsField {
    Host,
    Port,
    Units,
    CoordFormat,
    /// The "[ Save ]" button: Enter applies like Ctrl+S.
    Save,
}

impl SettingsField {
    pub const ALL: &[SettingsField] = &[
        SettingsField::Host,
        SettingsField::Port,
        SettingsField::Units,
        SettingsField::CoordFormat,
        SettingsField::Save,
    ];

    fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    fn prev(self) -> Self {
        let all = Self::ALL;
        let idx = all.iter().position(|f| *f == self).unwrap_or(0);
        all[(idx + all.len() - 1) % all.len()]
    }
}

/// Longest host name accepted by the settings dialog (DNS limit).
const MAX_HOST_LEN: usize = 253;

/// Validate a port typed into the settings dialog.
pub fn validate_port(text: &str) -> Result<u16, String> {
    if text.is_empty() {
        return Err("Port is empty: enter 1-65535".to_string());
    }
    if !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err("Port must be digits only".to_string());
    }
    match text.parse::<u32>() {
        Ok(p) if (1..=65535).contains(&p) => Ok(p as u16),
        _ => Err(format!("Invalid port {}: must be 1-65535", text)),
    }
}

impl App {
    // ------------------------------------------------------------------
    // Settings dialog
    //
    // Opening snapshots the current values. Units/coords preview live.
    // Esc while editing a text field cancels that field; Esc otherwise
    // discards every change and closes. Ctrl+S (or Enter on [Save])
    // applies: saves the config file and reconnects if host/port changed.
    // ------------------------------------------------------------------

    pub fn open_settings(&mut self) {
        self.show_settings = true;
        self.settings_editing = false;
        self.settings_field = SettingsField::Host;
        self.settings_message.clear();
        self.settings_snapshot = Some(self.current_config());
    }

    /// Discard all changes made in the dialog and close it.
    pub fn cancel_settings(&mut self) {
        if let Some(old) = self.settings_snapshot.take() {
            let changed = old != self.current_config();
            self.host = old.host;
            self.port = old.port;
            self.units = old.units;
            self.coord_format = old.coord_format;
            if changed {
                self.set_status("Settings changes discarded");
            }
        }
        self.close_settings();
    }

    /// Apply the dialog's values: persist them and reconnect if the gpsd
    /// address changed.
    pub fn apply_settings(&mut self) {
        let old = self
            .settings_snapshot
            .take()
            .unwrap_or_else(|| self.current_config());
        self.close_settings();

        let new = self.current_config();
        let mut msg = Vec::new();
        if old.host != new.host || old.port != new.port {
            self.reconnect_requested = true;
            msg.push(format!("Reconnecting to {}:{}", new.host, new.port));
        } else {
            msg.push("Settings applied".to_string());
        }
        if let Some(path) = &self.config_path {
            match config::save(path, &new) {
                Ok(()) => msg.push(format!("saved to {}", path.display())),
                Err(e) => msg.push(format!("NOT saved ({}: {})", path.display(), e)),
            }
        }
        self.set_status(msg.join("; "));
    }

    fn close_settings(&mut self) {
        self.show_settings = false;
        self.settings_editing = false;
        self.settings_message.clear();
    }

    /// Validate and store the text field being edited. On error the
    /// message is shown in the dialog and editing continues.
    fn commit_settings_edit(&mut self) -> bool {
        let result = match self.settings_field {
            SettingsField::Host => {
                let host = self.settings_edit_buf.trim();
                if host.is_empty() {
                    Err("Host must not be empty".to_string())
                } else {
                    self.host = host.to_string();
                    Ok(())
                }
            }
            SettingsField::Port => validate_port(&self.settings_edit_buf).map(|p| self.port = p),
            _ => Ok(()),
        };
        match result {
            Ok(()) => {
                self.settings_editing = false;
                self.settings_message.clear();
                true
            }
            Err(e) => {
                self.settings_message = e;
                false
            }
        }
    }

    pub(super) fn handle_settings_key(&mut self, key: crossterm::event::KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('s') {
            if !self.settings_editing || self.commit_settings_edit() {
                self.apply_settings();
            }
            return;
        }

        if self.settings_editing {
            self.handle_settings_text_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc => self.cancel_settings(),
            KeyCode::Up | KeyCode::BackTab => self.settings_field = self.settings_field.prev(),
            KeyCode::Down | KeyCode::Tab => self.settings_field = self.settings_field.next(),
            KeyCode::Enter => match self.settings_field {
                SettingsField::Host => {
                    self.settings_editing = true;
                    self.settings_edit_buf = self.host.clone();
                }
                SettingsField::Port => {
                    self.settings_editing = true;
                    self.settings_edit_buf = self.port.to_string();
                }
                SettingsField::Units => self.units = self.units.next(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.next(),
                SettingsField::Save => self.apply_settings(),
            },
            KeyCode::Left => match self.settings_field {
                SettingsField::Units => self.units = self.units.prev(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.prev(),
                _ => {}
            },
            KeyCode::Right => match self.settings_field {
                SettingsField::Units => self.units = self.units.next(),
                SettingsField::CoordFormat => self.coord_format = self.coord_format.next(),
                _ => {}
            },
            _ => {}
        }
    }

    fn handle_settings_text_key(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.settings_editing = false;
                self.settings_message.clear();
            }
            KeyCode::Enter => {
                self.commit_settings_edit();
            }
            KeyCode::Backspace => {
                self.settings_edit_buf.pop();
                self.settings_message.clear();
            }
            KeyCode::Char(_)
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {}
            KeyCode::Char(c) => match self.settings_field {
                SettingsField::Port => {
                    if !c.is_ascii_digit() {
                        self.settings_message = format!("Port accepts digits only ('{}')", c);
                    } else if self.settings_edit_buf.len() >= 5 {
                        self.settings_message = "Port is at most 5 digits (1-65535)".to_string();
                    } else {
                        self.settings_edit_buf.push(c);
                        self.settings_message = match validate_port(&self.settings_edit_buf) {
                            Ok(_) => String::new(),
                            Err(e) => e,
                        };
                    }
                }
                _ => {
                    if c.is_whitespace() || c.is_control() {
                        self.settings_message = "Host cannot contain spaces".to_string();
                    } else if self.settings_edit_buf.len() >= MAX_HOST_LEN {
                        self.settings_message = "Host name too long".to_string();
                    } else {
                        self.settings_edit_buf.push(c);
                        self.settings_message.clear();
                    }
                }
            },
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_util::*;
    use crate::app::{ActiveTab, CoordFormat, UnitSystem};

    // === Settings overlay ===

    #[test]
    fn test_settings_field_navigation() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open settings
        assert_eq!(app.settings_field, SettingsField::Host);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Port);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Units);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::CoordFormat);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Save);

        app.handle_event(key_event(KeyCode::Down));
        assert_eq!(app.settings_field, SettingsField::Host); // wraps

        app.handle_event(key_event(KeyCode::Up));
        assert_eq!(app.settings_field, SettingsField::Save); // wraps back
    }

    #[test]
    fn test_settings_edit_host() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        assert_eq!(app.settings_field, SettingsField::Host);

        app.handle_event(key_event(KeyCode::Enter)); // start editing
        assert!(app.settings_editing);
        assert_eq!(app.settings_edit_buf, "localhost");

        // Clear and type new host
        for _ in 0..9 {
            app.handle_event(key_event(KeyCode::Backspace));
        }
        for c in "10.0.0.1".chars() {
            app.handle_event(key_event(KeyCode::Char(c)));
        }
        assert_eq!(app.settings_edit_buf, "10.0.0.1");

        app.handle_event(key_event(KeyCode::Enter)); // apply
        assert!(!app.settings_editing);
        assert_eq!(app.host, "10.0.0.1");
    }

    #[test]
    fn test_settings_edit_cancel() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Enter)); // start editing host
        app.handle_event(key_event(KeyCode::Backspace));
        app.handle_event(key_event(KeyCode::Esc)); // cancel
        assert!(!app.settings_editing);
        assert_eq!(app.host, "localhost"); // unchanged
    }

    #[test]
    fn test_settings_cycle_units() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        // Navigate to Units field
        app.handle_event(key_event(KeyCode::Down)); // Port
        app.handle_event(key_event(KeyCode::Down)); // Units
        assert_eq!(app.settings_field, SettingsField::Units);

        app.handle_event(key_event(KeyCode::Enter));
        assert_eq!(app.units, UnitSystem::Imperial);

        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Nautical);
    }

    #[test]
    fn test_settings_cycle_coord_format() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        // Navigate to CoordFormat
        app.handle_event(key_event(KeyCode::Down)); // Port
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Down)); // CoordFormat
        assert_eq!(app.settings_field, SettingsField::CoordFormat);

        app.handle_event(key_event(KeyCode::Enter));
        assert_eq!(app.coord_format, CoordFormat::DMS);
    }

    #[test]
    fn test_settings_ctrl_s() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open
        assert!(app.show_settings);

        // Nothing changed: apply closes without reconnecting.
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert!(!app.reconnect_requested);
        assert!(app.status_message.contains("applied"));
    }

    #[test]
    fn test_settings_ctrl_s_reconnects_when_address_changed() {
        let mut app = new_app();
        type_port(&mut app, "3000");
        app.handle_event(key_event(KeyCode::Enter)); // commit field
        assert_eq!(app.port, 3000);
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert!(app.reconnect_requested);
        assert!(
            app.status_message
                .contains("Reconnecting to localhost:3000"),
            "{}",
            app.status_message
        );
    }

    #[test]
    fn test_settings_ctrl_s_while_editing_commits_field() {
        let mut app = new_app();
        type_port(&mut app, "4000");
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(!app.show_settings);
        assert_eq!(app.port, 4000);
        assert!(app.reconnect_requested);
    }

    #[test]
    fn test_settings_ctrl_s_with_invalid_port_stays_open() {
        let mut app = new_app();
        type_port(&mut app, "0");
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.show_settings);
        assert!(app.settings_editing);
        assert_eq!(app.port, 2947);
        assert!(!app.reconnect_requested);
        assert!(app.settings_message.contains("1-65535"));
    }

    #[test]
    fn test_settings_port_digits_only() {
        let mut app = new_app();
        type_port(&mut app, "12a3");
        assert_eq!(app.settings_edit_buf, "123");
        // The rejection is visible (it was replaced by the later valid digit,
        // so check by typing a letter last).
        app.handle_event(key_event(KeyCode::Char('x')));
        assert!(
            app.settings_message.contains("digits only"),
            "{}",
            app.settings_message
        );
        assert_eq!(app.settings_edit_buf, "123");
    }

    #[test]
    fn test_settings_port_max_five_digits() {
        let mut app = new_app();
        type_port(&mut app, "123456");
        assert_eq!(app.settings_edit_buf, "12345");
        assert!(app.settings_message.contains("5 digits"));
    }

    #[test]
    fn test_settings_port_out_of_range_rejected_on_enter() {
        for bad in ["0", "65536", "99999", ""] {
            let mut app = new_app();
            type_port(&mut app, bad);
            app.handle_event(key_event(KeyCode::Enter));
            assert!(app.settings_editing, "{bad}: should stay editing");
            assert_eq!(app.port, 2947, "{bad}");
            assert!(!app.settings_message.is_empty(), "{bad}");
        }
        let mut app = new_app();
        type_port(&mut app, "65535");
        app.handle_event(key_event(KeyCode::Enter));
        assert!(!app.settings_editing);
        assert_eq!(app.port, 65535);
        assert!(app.settings_message.is_empty());
    }

    #[test]
    fn test_validate_port() {
        assert_eq!(validate_port("1"), Ok(1));
        assert_eq!(validate_port("65535"), Ok(65535));
        assert!(validate_port("0").is_err());
        assert!(validate_port("65536").is_err());
        assert!(validate_port("").is_err());
        assert!(validate_port("-1").is_err());
        assert!(validate_port("12ab").is_err());
        assert!(validate_port("99999999999999999999").is_err());
    }

    #[test]
    fn test_settings_empty_host_rejected() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Enter));
        for _ in 0..20 {
            app.handle_event(key_event(KeyCode::Backspace));
        }
        app.handle_event(key_event(KeyCode::Enter));
        assert!(app.settings_editing);
        assert_eq!(app.host, "localhost");
        assert!(app.settings_message.contains("empty"));
        // Spaces are rejected visibly.
        app.handle_event(key_event(KeyCode::Char(' ')));
        assert!(app.settings_edit_buf.is_empty());
        assert!(app.settings_message.contains("spaces"));
    }

    #[test]
    fn test_settings_left_right_cycle_both_ways() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Down));
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.units, UnitSystem::Nautical);
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.units, UnitSystem::Imperial);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Nautical);

        app.handle_event(key_event(KeyCode::Down)); // Coords
        app.handle_event(key_event(KeyCode::Left));
        assert_eq!(app.coord_format, CoordFormat::DDM);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.coord_format, CoordFormat::DD);
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.coord_format, CoordFormat::DMS);
    }

    #[test]
    fn test_settings_esc_discards_all_changes() {
        let mut app = new_app();
        type_port(&mut app, "5000");
        app.handle_event(key_event(KeyCode::Enter)); // commit port
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Right));
        assert_eq!(app.units, UnitSystem::Imperial); // live preview
        app.handle_event(key_event(KeyCode::Esc));
        assert!(!app.show_settings);
        assert_eq!(app.port, 2947);
        assert_eq!(app.units, UnitSystem::Metric);
        assert!(!app.reconnect_requested);
        assert!(app.status_message.contains("discarded"));
    }

    #[test]
    fn test_settings_esc_while_editing_cancels_field_only() {
        let mut app = new_app();
        type_port(&mut app, "5000");
        app.handle_event(key_event(KeyCode::Esc));
        assert!(app.show_settings);
        assert!(!app.settings_editing);
        assert_eq!(app.port, 2947);
        app.handle_event(key_event(KeyCode::Esc));
        assert!(!app.show_settings);
    }

    #[test]
    fn test_settings_enter_on_save_applies() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Up)); // wraps to Save
        assert_eq!(app.settings_field, SettingsField::Save);
        app.handle_event(key_event(KeyCode::Enter));
        assert!(!app.show_settings);
        assert!(app.settings_snapshot.is_none());
    }

    #[test]
    fn test_settings_apply_saves_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub/config.toml");
        let mut app = new_app();
        app.config_path = Some(path.clone());
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event(KeyCode::Down));
        app.handle_event(key_event(KeyCode::Down)); // Units
        app.handle_event(key_event(KeyCode::Right));
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(app.status_message.contains("saved to"));
        let (cfg, outcome) = config::load(&path);
        assert_eq!(outcome, config::LoadOutcome::Loaded);
        assert_eq!(cfg.units, UnitSystem::Imperial);
        assert_eq!(cfg.host, "localhost");
    }

    #[test]
    fn test_settings_save_failure_reported() {
        let dir = tempfile::tempdir().unwrap();
        // Parent "directory" is a file, so saving must fail.
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "x").unwrap();
        let mut app = new_app();
        app.config_path = Some(blocker.join("config.toml"));
        app.handle_event(key_event(KeyCode::Char('s')));
        app.handle_event(key_event_mod(KeyCode::Char('s'), KeyModifiers::CONTROL));
        assert!(
            app.status_message.contains("NOT saved"),
            "{}",
            app.status_message
        );
        assert!(!app.show_settings);
    }

    #[test]
    fn test_settings_digits_do_not_switch_tabs() {
        let mut app = new_app();
        type_port(&mut app, "3");
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        app.handle_event(key_event(KeyCode::Esc));
        // Navigation mode in settings also swallows digits.
        app.handle_event(key_event(KeyCode::Char('4')));
        assert_eq!(app.active_tab, ActiveTab::Dashboard);
        assert!(app.show_settings);
    }

    #[test]
    fn test_settings_blocks_global_keys() {
        let mut app = new_app();
        app.handle_event(key_event(KeyCode::Char('s'))); // open settings
        assert!(app.show_settings);

        // 'q' should NOT quit while settings is open
        app.handle_event(key_event(KeyCode::Char('q')));
        assert!(!app.should_quit);
        assert!(app.show_settings);
    }
}
