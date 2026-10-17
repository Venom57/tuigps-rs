//! Property tests (plan T7): the gpsd parser, `apply`, the App event path and
//! the renderer never panic on arbitrary input, and applying messages never
//! produces infinite values. Case counts are kept small so the suite stays
//! fast; failure persistence is off so nothing is written into the repo.

use std::sync::OnceLock;

use proptest::prelude::*;
use proptest::test_runner::Config;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Map, Number, Value};

use tuigps_rs::app::{ActiveTab, App};
use tuigps_rs::data_model::GPSData;
use tuigps_rs::gpsd_client::{GpsdEvent, GpsdMessage, classify_line};
use tuigps_rs::position_hold::PositionHold;
use tuigps_rs::ui;

fn config(cases: u32) -> Config {
    Config {
        cases,
        failure_persistence: None,
        ..Config::default()
    }
}

/// Message classes with the keys the client reads for each, so generated
/// objects exercise real code paths instead of mostly-ignored keys.
const CLASSES: &[(&str, &[&str])] = &[
    (
        "TPV",
        &[
            "mode",
            "status",
            "lat",
            "lon",
            "altHAE",
            "altMSL",
            "geoidSep",
            "speed",
            "track",
            "climb",
            "magtrack",
            "magvar",
            "leapseconds",
            "eph",
            "epv",
            "ept",
            "eps",
            "epd",
            "epc",
            "epx",
            "epy",
            "sep",
            "ecefx",
            "ecefy",
            "ecefz",
            "ecefvx",
            "ecefvy",
            "ecefvz",
            "time",
        ],
    ),
    (
        "SKY",
        &[
            "hdop",
            "vdop",
            "pdop",
            "gdop",
            "tdop",
            "xdop",
            "ydop",
            "satellites",
            "nSat",
        ],
    ),
    (
        "PPS",
        &[
            "real_sec",
            "real_nsec",
            "clock_sec",
            "clock_nsec",
            "precision",
            "qErr",
        ],
    ),
    (
        "TOFF",
        &["real_sec", "real_nsec", "clock_sec", "clock_nsec"],
    ),
    ("DEVICES", &["devices"]),
    (
        "DEVICE",
        &[
            "path",
            "driver",
            "subtype",
            "bps",
            "cycle",
            "mincycle",
            "activated",
            "native",
        ],
    ),
    ("VERSION", &["release", "proto_major", "proto_minor"]),
    ("WATCH", &["enable", "json", "nmea"]),
    ("ERROR", &["message"]),
    ("GST", &["lat", "lon", "rms"]),
    ("POLL", &["tpv", "sky"]),
    ("", &["mode", "lat"]),
];

const SAT_KEYS: &[&str] = &[
    "PRN", "gnssid", "svid", "el", "az", "ss", "used", "sigid", "health", "freqid",
];

/// Integer edge cases: type limits and values that wrap when cast.
const EDGE_INTS: &[i64] = &[
    i64::MIN,
    i64::MAX,
    -1,
    0,
    256,
    259,
    65_536 + 7,
    (1 << 32) + 18,
];

fn leaf() -> impl Strategy<Value = Value> {
    prop_oneof![
        1 => Just(Value::Null),
        1 => any::<bool>().prop_map(Value::Bool),
        1 => any::<i64>().prop_map(Value::from),
        1 => Just(Value::from(u64::MAX)),
        3 => prop::sample::select(EDGE_INTS).prop_map(Value::from),
        2 => (-500i64..500).prop_map(Value::from),
        1 => any::<f64>().prop_map(|f| Number::from_f64(f).map_or(Value::Null, Value::Number)),
        2 => (-1000.0f64..1000.0).prop_map(Value::from),
        1 => "\\PC{0,24}".prop_map(Value::String),
        // Plausible timestamps, valid and not
        1 => prop::sample::select(&[
            "2024-05-12T14:05:01.000Z",
            "9999-12-31T23:59:59.999Z",
            "0000-01-01T00:00:00Z",
            "2024-13-45T99:99:99Z",
        ][..])
        .prop_map(|s| Value::String(s.to_string())),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    leaf().prop_recursive(3, 24, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::btree_map("[a-z]{1,6}", inner, 0..6)
                .prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn object(fields: Vec<(&str, Value)>) -> Value {
    let mut m = Map::new();
    for (k, v) in fields {
        m.insert(k.to_string(), v);
    }
    Value::Object(m)
}

fn satellite() -> impl Strategy<Value = Value> {
    prop::collection::vec((prop::sample::select(SAT_KEYS), leaf()), 0..SAT_KEYS.len())
        .prop_map(object)
}

/// A JSON object shaped like a gpsd message with random field values.
fn gpsd_object() -> impl Strategy<Value = Value> {
    (prop::sample::select(CLASSES), prop::bool::weighted(0.1)).prop_flat_map(
        |((class, keys), odd_class)| {
            let field = (
                prop::sample::select(keys),
                prop_oneof![
                    6 => leaf(),
                    1 => value(),
                    2 => prop::collection::vec(satellite(), 0..8).prop_map(Value::Array),
                    1 => prop::collection::vec(satellite(), 0..3).prop_map(Value::Array),
                ],
            );
            let class = if odd_class {
                leaf().boxed()
            } else {
                Just(Value::String(class.to_string())).boxed()
            };
            (class, prop::collection::vec(field, 0..12)).prop_map(|(class, mut fields)| {
                fields.push(("class", class));
                object(fields)
            })
        },
    )
}

/// Every f64 in the model: NaN means "unknown", but never infinite.
fn assert_no_infinities(d: &GPSData) {
    let mut values = vec![
        ("last_seen", d.last_seen),
        ("last_tpv", d.last_tpv),
        ("latitude", d.latitude),
        ("longitude", d.longitude),
        ("alt_hae", d.alt_hae),
        ("alt_msl", d.alt_msl),
        ("geoid_sep", d.geoid_sep),
        ("speed", d.speed),
        ("track", d.track),
        ("climb", d.climb),
        ("magtrack", d.magtrack),
        ("magvar", d.magvar),
        ("ecefx", d.ecefx),
        ("ecefy", d.ecefy),
        ("ecefz", d.ecefz),
        ("ecefvx", d.ecefvx),
        ("ecefvy", d.ecefvy),
        ("ecefvz", d.ecefvz),
        ("hdop", d.dop.hdop),
        ("vdop", d.dop.vdop),
        ("pdop", d.dop.pdop),
        ("gdop", d.dop.gdop),
        ("tdop", d.dop.tdop),
        ("xdop", d.dop.xdop),
        ("ydop", d.dop.ydop),
        ("eph", d.errors.eph),
        ("epv", d.errors.epv),
        ("ept", d.errors.ept),
        ("eps", d.errors.eps),
        ("epd", d.errors.epd),
        ("epc", d.errors.epc),
        ("epx", d.errors.epx),
        ("epy", d.errors.epy),
        ("sep", d.errors.sep),
        ("cycle", d.device.cycle),
        ("mincycle", d.device.mincycle),
        ("toff_armed_offset", d.toff_armed_offset),
        ("toff_armed_sys_time", d.toff_armed_sys_time),
        ("pps_offset_us", d.pps_offset_us()),
    ];
    for s in &d.satellites {
        values.extend([("el", s.elevation), ("az", s.azimuth), ("ss", s.snr)]);
    }
    values.extend(d.toff_samples.iter().map(|&t| ("toff_sample", t)));
    for (name, v) in values {
        assert!(!v.is_infinite(), "{name} is {v}");
    }
    assert_eq!(
        d.satellites_used as usize,
        d.satellites.iter().filter(|s| s.used).count()
    );
    if d.has_fix() {
        assert!(d.latitude.is_finite() && d.longitude.is_finite());
    }
}

/// App with realistic data from the fixtures, so every panel has content.
fn populated_app() -> App {
    let mut app = App::new("localhost".into(), 2947);
    app.handle_tagged_event((0, GpsdEvent::Connected));
    for name in ["session.jsonl", "sky.jsonl"] {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(path).unwrap();
        for line in text.lines().take(if name == "sky.jsonl" { 1 } else { 100 }) {
            if let Some(event) = classify_line(line, 1_715_522_701.0) {
                app.handle_tagged_event((0, event));
            }
        }
    }
    app
}

/// GPSData of `populated_app`, built once.
fn populated_data() -> GPSData {
    static DATA: OnceLock<GPSData> = OnceLock::new();
    DATA.get_or_init(|| populated_app().gps_data).clone()
}

fn render(app: &App, w: u16, h: u16) {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| ui::render(f, app)).unwrap();
}

proptest! {
    #![proptest_config(config(512))]

    #[test]
    fn parse_never_panics_on_arbitrary_strings(s in any::<String>()) {
        let _ = GpsdMessage::parse(&s, 1.0);
        let _ = classify_line(&s, 1.0);
    }

    #[test]
    fn parse_never_panics_on_json_like_strings(s in r#"[{}\[\]":,0-9a-zA-Z.eE+\- \r\n\t\\]{0,80}"#) {
        let _ = GpsdMessage::parse(&s, 1.0);
        let _ = classify_line(&s, 1.0);
    }
}

proptest! {
    #![proptest_config(config(1024))]

    #[test]
    fn apply_never_panics_or_produces_infinities(obj in gpsd_object()) {
        let line = obj.to_string();
        let mut fresh = GPSData::default();
        let mut populated = populated_data();
        if let Some(msg) = GpsdMessage::parse(&line, 1_715_522_702.0) {
            msg.apply(&mut fresh);
            msg.apply(&mut populated);
        }
        assert_no_infinities(&fresh);
        assert_no_infinities(&populated);
    }
}

proptest! {
    #![proptest_config(config(48))]

    /// Random messages through the full App path (TOFF, hold, logging off,
    /// clock-sync queueing), then a render of every tab.
    #[test]
    fn app_events_then_render_never_panic(
        objs in prop::collection::vec(gpsd_object(), 1..6),
        armed in any::<bool>(),
    ) {
        let mut app = populated_app();
        app.armed_clock_set = armed;
        app.armed_toff = true;
        app.position_hold = Some(PositionHold::new());
        for (i, obj) in objs.iter().enumerate() {
            if let Some(event) = classify_line(&obj.to_string(), 1_715_522_702.0 + i as f64) {
                app.handle_tagged_event((0, event));
            }
        }
        assert_no_infinities(&app.gps_data);
        app.update_staleness(1_715_522_720.0);
        for &tab in ActiveTab::ALL {
            app.active_tab = tab;
            render(&app, 100, 30);
        }
    }

    #[test]
    fn render_never_panics_at_any_size(
        w in 0u16..=300,
        h in 0u16..=100,
        tab in prop::sample::select(ActiveTab::ALL),
        settings in any::<bool>(),
        populated in any::<bool>(),
    ) {
        let mut app = if populated {
            populated_app()
        } else {
            App::new("localhost".into(), 2947)
        };
        app.active_tab = tab;
        app.show_settings = settings;
        render(&app, w, h);
    }
}
