//! Persistent user settings stored as TOML.
//!
//! Location: `$XDG_CONFIG_HOME/tuigps-rs/config.toml`, falling back to
//! `$HOME/.config/tuigps-rs/config.toml`, or an explicit `--config <path>`.
//! A missing or unreadable file never stops the program: defaults are used
//! and a message explains why.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::app::{CoordFormat, UnitSystem};

pub const DEFAULT_HOST: &str = "localhost";
pub const DEFAULT_PORT: u16 = 2947;

const APP_DIR: &str = "tuigps-rs";
const FILE_NAME: &str = "config.toml";

/// Everything the settings overlay can change. Missing keys take their
/// default; unknown keys are ignored so older binaries can read newer files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub units: UnitSystem,
    pub coord_format: CoordFormat,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            host: DEFAULT_HOST.to_string(),
            port: DEFAULT_PORT,
            units: UnitSystem::Metric,
            coord_format: CoordFormat::DD,
        }
    }
}

/// What happened when the config file was read.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadOutcome {
    /// File parsed successfully.
    Loaded,
    /// No file yet; defaults are used.
    Missing,
    /// File exists but could not be read or parsed; defaults are used.
    Invalid(String),
}

/// Resolve the default config file path from the environment.
pub fn default_path() -> Option<PathBuf> {
    resolve_path(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

/// Pure path resolution (testable without touching the environment).
/// Per the XDG spec, a relative `XDG_CONFIG_HOME` is ignored.
pub fn resolve_path(xdg_config_home: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let base = xdg_config_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".config"))
        })?;
    Some(base.join(APP_DIR).join(FILE_NAME))
}

/// Parse config text. Invalid values (e.g. port 0) are rejected.
pub fn parse(text: &str) -> Result<Config, String> {
    let cfg: Config = toml::from_str(text).map_err(|e| e.message().to_string())?;
    if cfg.port == 0 {
        return Err("port must be 1-65535".to_string());
    }
    if cfg.host.trim().is_empty() {
        return Err("host must not be empty".to_string());
    }
    Ok(cfg)
}

/// Load the config file; never fails. On any problem the defaults are
/// returned together with the reason.
pub fn load(path: &Path) -> (Config, LoadOutcome) {
    match std::fs::read_to_string(path) {
        Ok(text) => match parse(&text) {
            Ok(cfg) => (cfg, LoadOutcome::Loaded),
            Err(e) => (Config::default(), LoadOutcome::Invalid(e)),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => (Config::default(), LoadOutcome::Missing),
        Err(e) => (Config::default(), LoadOutcome::Invalid(e.to_string())),
    }
}

pub fn to_toml(cfg: &Config) -> String {
    // Serializing a flat struct of strings/ints/enums cannot fail.
    let body = toml::to_string(cfg).unwrap_or_default();
    format!("# tuigps-rs settings (written by the settings dialog)\n{body}")
}

/// Write the config atomically (temp file + rename), creating the parent
/// directory if needed.
pub fn save(path: &Path, cfg: &Config) -> io::Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, to_toml(cfg))?;
    std::fs::rename(&tmp, path)
}

/// Settings in effect at startup and a status line describing where they
/// came from.
#[derive(Debug, Clone, PartialEq)]
pub struct Startup {
    pub config: Config,
    pub config_path: Option<PathBuf>,
    pub message: Option<String>,
}

/// Load the config (if a path is known) and apply CLI overrides. Explicit
/// `--host` / `--port` always win over the file.
pub fn startup(
    config_path: Option<PathBuf>,
    cli_host: Option<String>,
    cli_port: Option<u16>,
) -> Startup {
    let (mut config, message) = match &config_path {
        Some(path) => {
            let (cfg, outcome) = load(path);
            let msg = match outcome {
                LoadOutcome::Loaded => None,
                LoadOutcome::Missing => Some(format!(
                    "No config file; using defaults (settings are saved to {})",
                    path.display()
                )),
                LoadOutcome::Invalid(e) => Some(format!(
                    "Config {} invalid ({}); using defaults",
                    path.display(),
                    e
                )),
            };
            (cfg, msg)
        }
        None => (
            Config::default(),
            Some("No config directory (HOME unset); settings will not be saved".to_string()),
        ),
    };
    apply_cli_overrides(&mut config, cli_host, cli_port);
    Startup {
        config,
        config_path,
        message,
    }
}

pub fn apply_cli_overrides(config: &mut Config, host: Option<String>, port: Option<u16>) {
    if let Some(h) = host {
        config.host = h;
    }
    if let Some(p) = port {
        config.port = p;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Config {
        Config {
            host: "gps.local".into(),
            port: 3000,
            units: UnitSystem::Nautical,
            coord_format: CoordFormat::DMS,
        }
    }

    #[test]
    fn resolve_prefers_xdg() {
        let p = resolve_path(Some("/x/cfg".into()), Some("/home/u".into())).unwrap();
        assert_eq!(p, PathBuf::from("/x/cfg/tuigps-rs/config.toml"));
    }

    #[test]
    fn resolve_falls_back_to_home() {
        let p = resolve_path(None, Some("/home/u".into())).unwrap();
        assert_eq!(p, PathBuf::from("/home/u/.config/tuigps-rs/config.toml"));
        // Relative XDG_CONFIG_HOME is ignored per the spec.
        let p = resolve_path(Some("rel/dir".into()), Some("/home/u".into())).unwrap();
        assert_eq!(p, PathBuf::from("/home/u/.config/tuigps-rs/config.toml"));
    }

    #[test]
    fn resolve_none_without_env() {
        assert_eq!(resolve_path(None, None), None);
        assert_eq!(resolve_path(Some("".into()), Some("".into())), None);
    }

    #[test]
    fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/dir/config.toml");
        save(&path, &sample()).unwrap();
        let (cfg, outcome) = load(&path);
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(cfg, sample());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("units = \"nautical\""), "{text}");
        assert!(text.contains("coord_format = \"dms\""), "{text}");
        assert!(!dir.path().join("nested/dir/config.toml.tmp").exists());
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let (cfg, outcome) = load(&dir.path().join("nope.toml"));
        assert_eq!(cfg, Config::default());
        assert_eq!(outcome, LoadOutcome::Missing);
    }

    #[test]
    fn corrupt_file_gives_defaults_and_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        for bad in [
            "this is = = not toml",
            "port = \"abc\"",
            "port = 70000",
            "port = 0",
            "units = \"furlongs\"",
            "host = \"\"",
            "\u{0}\u{1}\u{2}",
        ] {
            std::fs::write(&path, bad).unwrap();
            let (cfg, outcome) = load(&path);
            assert_eq!(cfg, Config::default(), "{bad}");
            assert!(matches!(outcome, LoadOutcome::Invalid(_)), "{bad}");
        }
    }

    #[test]
    fn unreadable_path_is_invalid_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be.
        let (cfg, outcome) = load(dir.path());
        assert_eq!(cfg, Config::default());
        assert!(matches!(outcome, LoadOutcome::Invalid(_)));
    }

    #[test]
    fn partial_file_keeps_defaults_for_missing_keys() {
        let cfg = parse("port = 4000\nfuture_key = true\n").unwrap();
        assert_eq!(cfg.port, 4000);
        assert_eq!(cfg.host, DEFAULT_HOST);
        assert_eq!(cfg.units, UnitSystem::Metric);
    }

    #[test]
    fn cli_overrides_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        save(&path, &sample()).unwrap();

        // No CLI args: file wins.
        let s = startup(Some(path.clone()), None, None);
        assert_eq!(s.config, sample());
        assert_eq!(s.message, None);

        // Explicit CLI args override host/port but keep the rest.
        let s = startup(Some(path.clone()), Some("10.0.0.9".into()), Some(9999));
        assert_eq!(s.config.host, "10.0.0.9");
        assert_eq!(s.config.port, 9999);
        assert_eq!(s.config.units, UnitSystem::Nautical);

        // Only one overridden.
        let s = startup(Some(path), None, Some(1234));
        assert_eq!(s.config.host, "gps.local");
        assert_eq!(s.config.port, 1234);
    }

    #[test]
    fn startup_messages() {
        let dir = tempfile::tempdir().unwrap();
        let s = startup(Some(dir.path().join("none.toml")), None, None);
        assert!(s.message.unwrap().contains("No config file"));

        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "[[[").unwrap();
        let s = startup(Some(bad), Some("h".into()), None);
        assert!(s.message.unwrap().contains("invalid"));
        assert_eq!(s.config.host, "h");

        let s = startup(None, None, None);
        assert!(s.message.unwrap().contains("not be saved"));
        assert_eq!(s.config, Config::default());
    }
}
