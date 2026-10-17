//! Key bindings: one table drives dispatch, the help overlay and the
//! status bar hints, so they cannot drift apart.

use crossterm::event::KeyCode;

use super::ActiveTab;

/// What a key does. Some actions are resolved further by the key code
/// (e.g. `ScrollLine` is Up or Down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleHelp,
    JumpTab,
    NextTab,
    PrevTab,
    Quit,
    Reconnect,
    OpenSettings,
    CycleUnits,
    OpenMaps,
    ToggleLog,
    ToggleHold,
    CloseOverlay,
    ScrollLine,
    ScrollPage,
    ScrollEnds,
    NmeaPause,
    NmeaFilter,
    NmeaClear,
    ToffArm,
    ToffClear,
    ClockSyncToggle,
    DeviceSelect,
    DeviceAdjust,
    DeviceApply,
    /// Documentation-only row (handled elsewhere, e.g. Ctrl+C).
    Info,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyBinding {
    /// Key codes that trigger the action (empty for documentation rows).
    pub keys: &'static [KeyCode],
    /// How the key is shown in help and the status bar.
    pub label: &'static str,
    /// Description for the help overlay.
    pub desc: &'static str,
    /// Short status-bar hint, if the binding is shown there.
    pub hint: Option<&'static str>,
    pub action: Action,
}

const fn kb(
    keys: &'static [KeyCode],
    label: &'static str,
    desc: &'static str,
    hint: Option<&'static str>,
    action: Action,
) -> KeyBinding {
    KeyBinding {
        keys,
        label,
        desc,
        hint,
        action,
    }
}

pub const GLOBAL_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('?')],
        "?",
        "Toggle this help",
        Some("help"),
        Action::ToggleHelp,
    ),
    kb(
        &[
            KeyCode::Char('1'),
            KeyCode::Char('2'),
            KeyCode::Char('3'),
            KeyCode::Char('4'),
            KeyCode::Char('5'),
        ],
        "1-5",
        "Jump to tab N",
        None,
        Action::JumpTab,
    ),
    kb(&[KeyCode::Tab], "Tab", "Next tab", None, Action::NextTab),
    kb(
        &[KeyCode::BackTab],
        "Shift+Tab",
        "Previous tab",
        None,
        Action::PrevTab,
    ),
    kb(
        &[KeyCode::Char('q')],
        "q",
        "Quit",
        Some("quit"),
        Action::Quit,
    ),
    kb(&[], "Ctrl+C", "Quit (works everywhere)", None, Action::Info),
    kb(
        &[KeyCode::Char('r')],
        "r",
        "Reconnect to gpsd",
        Some("reconnect"),
        Action::Reconnect,
    ),
    kb(
        &[KeyCode::Char('s')],
        "s",
        "Settings (host/port/units)",
        Some("settings"),
        Action::OpenSettings,
    ),
    kb(
        &[KeyCode::Char('u')],
        "u",
        "Cycle units (not saved)",
        Some("units"),
        Action::CycleUnits,
    ),
    kb(
        &[KeyCode::Char('m')],
        "m",
        "Open position in Maps",
        Some("maps"),
        Action::OpenMaps,
    ),
    kb(
        &[KeyCode::Char('l')],
        "l",
        "Start/stop GPX logging",
        Some("log"),
        Action::ToggleLog,
    ),
    kb(
        &[KeyCode::Char('h')],
        "h",
        "Start/stop position hold",
        Some("hold"),
        Action::ToggleHold,
    ),
    kb(
        &[KeyCode::Esc],
        "Esc",
        "Close overlay / dismiss",
        None,
        Action::CloseOverlay,
    ),
];

const SCROLL_LINE: KeyBinding = kb(
    &[KeyCode::Up, KeyCode::Down],
    "\u{2191}/\u{2193}",
    "Scroll one line",
    Some("scroll"),
    Action::ScrollLine,
);
const SCROLL_PAGE: KeyBinding = kb(
    &[KeyCode::PageUp, KeyCode::PageDown],
    "PgUp/PgDn",
    "Scroll one page",
    None,
    Action::ScrollPage,
);

const SATELLITE_BINDINGS: &[KeyBinding] = &[
    SCROLL_LINE,
    SCROLL_PAGE,
    kb(
        &[KeyCode::Home, KeyCode::End],
        "Home/End",
        "First / last satellite",
        None,
        Action::ScrollEnds,
    ),
];

const NMEA_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('p')],
        "p",
        "Pause / resume the stream",
        Some("pause"),
        Action::NmeaPause,
    ),
    kb(
        &[KeyCode::Char('f')],
        "f",
        "Cycle sentence filter",
        Some("filter"),
        Action::NmeaFilter,
    ),
    kb(
        &[KeyCode::Char('c')],
        "c",
        "Clear buffer",
        Some("clear"),
        Action::NmeaClear,
    ),
    kb(
        &[KeyCode::Up, KeyCode::Down],
        "\u{2191}/\u{2193}",
        "Scroll back / forward",
        None,
        Action::ScrollLine,
    ),
    SCROLL_PAGE,
    kb(
        &[KeyCode::Home, KeyCode::End],
        "Home/End",
        "Oldest / follow newest",
        None,
        Action::ScrollEnds,
    ),
];

const TIMING_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Char('a')],
        "a",
        "Arm one-shot TOFF capture",
        Some("arm"),
        Action::ToffArm,
    ),
    kb(
        &[KeyCode::Char('c')],
        "c",
        "Clear TOFF data",
        Some("clear"),
        Action::ToffClear,
    ),
    kb(
        &[KeyCode::Char('k')],
        "k",
        "Arm clock sync (next fix)",
        Some("clock"),
        Action::ClockSyncToggle,
    ),
];

const DEVICE_BINDINGS: &[KeyBinding] = &[
    kb(
        &[KeyCode::Up, KeyCode::Down],
        "\u{2191}/\u{2193}",
        "Select control",
        Some("select"),
        Action::DeviceSelect,
    ),
    kb(
        &[KeyCode::Left, KeyCode::Right],
        "\u{2190}/\u{2192}",
        "Change value (* = pending)",
        Some("adjust"),
        Action::DeviceAdjust,
    ),
    kb(
        &[KeyCode::Enter],
        "Enter",
        "Apply (asks y/n first)",
        Some("apply"),
        Action::DeviceApply,
    ),
    kb(
        &[],
        "y/other",
        "Confirm / cancel prompt",
        None,
        Action::Info,
    ),
];

/// Keys shown in the settings dialog (dispatch is in the settings handler,
/// which depends on whether a text field is being edited).
pub const SETTINGS_BINDINGS: &[KeyBinding] = &[
    kb(&[], "\u{2191}/\u{2193}", "move", None, Action::Info),
    kb(&[], "Enter", "edit / cycle / save", None, Action::Info),
    kb(&[], "\u{2190}/\u{2192}", "cycle value", None, Action::Info),
    kb(&[], "Ctrl+S", "apply & save", None, Action::Info),
    kb(
        &[],
        "Esc",
        "cancel edit; again: discard & close",
        None,
        Action::Info,
    ),
];

/// Tab-specific bindings for `tab`.
pub fn tab_bindings(tab: ActiveTab) -> &'static [KeyBinding] {
    match tab {
        ActiveTab::Dashboard => &[],
        ActiveTab::Satellites => SATELLITE_BINDINGS,
        ActiveTab::Timing => TIMING_BINDINGS,
        ActiveTab::Device => DEVICE_BINDINGS,
        ActiveTab::Nmea => NMEA_BINDINGS,
    }
}

/// Find the binding for `code` on `tab`. Global bindings take precedence;
/// a unit test guarantees no tab binding collides with a global one.
pub fn lookup_binding(tab: ActiveTab, code: KeyCode) -> Option<&'static KeyBinding> {
    GLOBAL_BINDINGS
        .iter()
        .chain(tab_bindings(tab))
        .find(|b| b.keys.contains(&code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_binding_collisions() {
        // Every key maps to at most one binding per tab, so global
        // precedence never hides a tab key.
        for tab in ActiveTab::ALL {
            let mut seen = Vec::new();
            for b in GLOBAL_BINDINGS.iter().chain(tab_bindings(*tab)) {
                for k in b.keys {
                    assert!(!seen.contains(k), "{tab:?}: {k:?} bound twice");
                    seen.push(*k);
                }
            }
        }
    }

    #[test]
    fn test_every_binding_documented() {
        for b in GLOBAL_BINDINGS
            .iter()
            .chain(ActiveTab::ALL.iter().flat_map(|t| tab_bindings(*t)))
            .chain(SETTINGS_BINDINGS)
        {
            assert!(!b.label.is_empty() && !b.desc.is_empty(), "{b:?}");
            assert!(
                b.keys.is_empty() == (b.action == Action::Info),
                "only Info rows may lack keys: {b:?}"
            );
        }
    }

    #[test]
    fn test_lookup_binding_scopes_tab_keys() {
        assert_eq!(
            lookup_binding(ActiveTab::Nmea, KeyCode::Char('c')).map(|b| b.action),
            Some(Action::NmeaClear)
        );
        assert_eq!(
            lookup_binding(ActiveTab::Timing, KeyCode::Char('c')).map(|b| b.action),
            Some(Action::ToffClear)
        );
        assert!(lookup_binding(ActiveTab::Dashboard, KeyCode::Char('c')).is_none());
        assert_eq!(
            lookup_binding(ActiveTab::Device, KeyCode::Char('q')).map(|b| b.action),
            Some(Action::Quit)
        );
    }
}
