//! gpsd client I/O against a mock gpsd on 127.0.0.1:0 (plan T4).
//!
//! Complements the single end-to-end unit test in `gpsd_client.rs` with:
//! the exact WATCH request, a fixture session streamed in odd-sized chunks,
//! the 64 KiB line limit on a live connection, reconnect timing (paused
//! tokio clock), prompt shutdown while connected, and generation tagging.
//! Never connects to a real gpsd.

use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use tuigps_rs::gpsd_client::{
    GpsdEvent, MAX_LINE_LEN, MsgClass, RECONNECT_DELAY, TaggedEvent, classify_line, gpsd_task,
};

const GEN: u64 = 42;

struct Harness {
    listener: TcpListener,
    rx: mpsc::Receiver<TaggedEvent>,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
}

async fn start() -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel(64);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(gpsd_task(
        "127.0.0.1".to_string(),
        port,
        GEN,
        tx,
        shutdown_rx,
    ));
    Harness {
        listener,
        rx,
        shutdown,
        task,
    }
}

/// Accept the client and consume its WATCH line; returns the socket and
/// the raw WATCH line.
async fn accept(listener: &TcpListener) -> (BufReader<TcpStream>, String) {
    let (sock, _) = listener.accept().await.unwrap();
    let mut sock = BufReader::new(sock);
    let mut line = String::new();
    sock.read_line(&mut line).await.unwrap();
    (sock, line)
}

/// Next event (real clock only), asserting the generation tag.
async fn recv(rx: &mut mpsc::Receiver<TaggedEvent>) -> GpsdEvent {
    let (generation, event) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("timed out waiting for event")
        .expect("channel closed");
    assert_eq!(generation, GEN, "event {event:?} has wrong generation");
    event
}

/// Next event without a timeout, for paused-clock tests (a pending timeout
/// would be auto-advanced while waiting on real socket I/O).
async fn recv_paused(rx: &mut mpsc::Receiver<TaggedEvent>) -> GpsdEvent {
    let (generation, event) = rx.recv().await.expect("channel closed");
    assert_eq!(generation, GEN);
    event
}

async fn stop(h: Harness) {
    h.shutdown.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(2), h.task)
        .await
        .expect("task did not stop after shutdown")
        .unwrap();
}

fn fixture(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// Comparable summary of an event.
fn describe(event: &GpsdEvent) -> String {
    match event {
        GpsdEvent::Connected => "Connected".into(),
        GpsdEvent::Message(m) => format!("{:?} {}", m.class, m.json),
        GpsdEvent::Error(e) => format!("Error {e}"),
        GpsdEvent::Nmea(s) => format!("Nmea {s}"),
    }
}

#[tokio::test]
async fn watch_request_is_one_json_line() {
    let mut h = start().await;
    let (_sock, line) = accept(&h.listener).await;
    assert!(line.ends_with('\n') && !line.ends_with("\r\n"), "{line:?}");
    let json = line
        .trim_end()
        .strip_prefix("?WATCH=")
        .unwrap_or_else(|| panic!("not a WATCH: {line:?}"));
    let watch: Value = serde_json::from_str(json).unwrap();
    for key in ["enable", "json", "nmea", "pps", "timing"] {
        assert_eq!(watch[key], Value::Bool(true), "{key} in {json}");
    }
    assert!(matches!(recv(&mut h.rx).await, GpsdEvent::Connected));
    stop(h).await;
}

#[tokio::test]
async fn streams_fixture_session_in_chunks() {
    let mut h = start().await;
    let (mut sock, _) = accept(&h.listener).await;
    assert!(matches!(recv(&mut h.rx).await, GpsdEvent::Connected));

    let mut payload = fixture("session.jsonl");
    payload.extend(fixture("crlf.jsonl"));
    // Invalid UTF-8 inside an NMEA sentence is replaced, not fatal
    payload.extend_from_slice(b"$GPTXT,01,01,02,\xff\xfeBAD*00\n");

    // Expected events: exactly what classify_line yields per line
    let expected: Vec<String> = String::from_utf8_lossy(&payload)
        .split('\n')
        .filter_map(|l| classify_line(l, 0.0))
        .map(|e| describe(&e))
        .collect();
    assert_eq!(expected.len(), 16);

    // Odd chunk size so lines are split across TCP writes
    for chunk in payload.chunks(37) {
        sock.write_all(chunk).await.unwrap();
        sock.flush().await.unwrap();
        tokio::task::yield_now().await;
    }

    let mut got = Vec::new();
    for _ in 0..expected.len() {
        got.push(describe(&recv(&mut h.rx).await));
    }
    assert_eq!(got, expected);
    assert!(got.last().unwrap().contains('\u{fffd}'));

    // Server goes away: one Error event, then the reconnect wait
    drop(sock);
    match recv(&mut h.rx).await {
        GpsdEvent::Error(e) => assert!(e.contains("closed"), "{e}"),
        other => panic!("expected Error, got {other:?}"),
    }
    stop(h).await;
}

/// A JSON TPV line of exactly `len` bytes (without the newline).
fn tpv_of_len(len: usize) -> Vec<u8> {
    let head = br#"{"class":"TPV","mode":3,"lat":1.0,"lon":2.0,"pad":""#;
    let tail = br#""}"#;
    let mut line = head.to_vec();
    line.resize(len - tail.len(), b'p');
    line.extend_from_slice(tail);
    assert_eq!(line.len(), len);
    line
}

#[tokio::test]
async fn line_limit_on_live_connection() {
    let mut h = start().await;
    let (mut sock, _) = accept(&h.listener).await;
    assert!(matches!(recv(&mut h.rx).await, GpsdEvent::Connected));

    // One byte over the limit: discarded; exactly at the limit: accepted;
    // a 1 MiB line: discarded. The connection stays up throughout.
    let mut payload = tpv_of_len(MAX_LINE_LEN + 1);
    payload.push(b'\n');
    payload.extend(tpv_of_len(MAX_LINE_LEN));
    payload.push(b'\n');
    payload.extend(vec![b'z'; 1 << 20]);
    payload.extend_from_slice(b"\n$GPGGA,after-oversized*00\n");
    sock.write_all(&payload).await.unwrap();

    match recv(&mut h.rx).await {
        GpsdEvent::Message(m) => {
            assert_eq!(m.class, MsgClass::Tpv);
            assert!(m.json["pad"].as_str().unwrap().len() > MAX_LINE_LEN - 100);
        }
        other => panic!("expected the at-limit TPV, got {other:?}"),
    }
    assert!(matches!(
        recv(&mut h.rx).await,
        GpsdEvent::Nmea(s) if s == "$GPGGA,after-oversized*00"
    ));

    // Still the same connection: more data flows without a reconnect
    sock.write_all(b"{\"class\":\"PPS\",\"real_sec\":5}\n")
        .await
        .unwrap();
    match recv(&mut h.rx).await {
        GpsdEvent::Message(m) => assert_eq!(m.class, MsgClass::Pps),
        other => panic!("expected PPS on the same connection, got {other:?}"),
    }
    assert!(h.rx.try_recv().is_err(), "no Error/Connected in between");
    stop(h).await;
}

#[tokio::test]
async fn shutdown_while_connected_is_prompt() {
    let mut h = start().await;
    let (_sock, _) = accept(&h.listener).await;
    assert!(matches!(recv(&mut h.rx).await, GpsdEvent::Connected));
    // Socket is open and idle: the task is blocked reading
    let t0 = std::time::Instant::now();
    stop(h).await;
    assert!(t0.elapsed() < Duration::from_millis(500));
}

#[tokio::test]
async fn task_ends_when_receiver_is_dropped() {
    let h = start().await;
    let (mut sock, _) = accept(&h.listener).await;
    drop(h.rx);
    // The next event cannot be delivered, so the task exits on its own
    sock.write_all(b"$GPGGA,1*00\n").await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), h.task)
        .await
        .expect("task kept running without a receiver")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn reconnects_after_delay_with_same_generation() {
    let mut h = start().await;
    let (sock, watch1) = accept(&h.listener).await;
    assert!(matches!(recv_paused(&mut h.rx).await, GpsdEvent::Connected));

    drop(sock);
    assert!(matches!(recv_paused(&mut h.rx).await, GpsdEvent::Error(_)));
    let t0 = tokio::time::Instant::now();

    // The only pending timer is the client's reconnect sleep, so the paused
    // clock advances by exactly that much before the second connection.
    let (mut sock, watch2) = accept(&h.listener).await;
    let waited = t0.elapsed();
    assert!(waited >= RECONNECT_DELAY, "reconnected after {waited:?}");
    assert!(waited < RECONNECT_DELAY + Duration::from_millis(100));
    assert_eq!(watch1, watch2, "WATCH is re-sent on reconnect");
    assert!(matches!(recv_paused(&mut h.rx).await, GpsdEvent::Connected));

    sock.write_all(b"{\"class\":\"TPV\",\"mode\":2}\n")
        .await
        .unwrap();
    assert!(matches!(
        recv_paused(&mut h.rx).await,
        GpsdEvent::Message(m) if m.class == MsgClass::Tpv
    ));

    h.shutdown.send(true).unwrap();
    h.task.await.unwrap();
}

#[tokio::test(start_paused = true)]
async fn connection_refused_retries_every_delay() {
    // Grab a free port, then close it so connecting is refused
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let (tx, mut rx) = mpsc::channel(8);
    let (shutdown, shutdown_rx) = watch::channel(false);
    let task = tokio::spawn(gpsd_task("127.0.0.1".into(), port, GEN, tx, shutdown_rx));

    let mut times = Vec::new();
    for _ in 0..3 {
        match recv_paused(&mut rx).await {
            GpsdEvent::Error(e) => assert!(!e.is_empty()),
            other => panic!("expected connect error, got {other:?}"),
        }
        times.push(tokio::time::Instant::now());
    }
    for pair in times.windows(2) {
        let gap = pair[1] - pair[0];
        assert!(gap >= RECONNECT_DELAY && gap < RECONNECT_DELAY + Duration::from_millis(100));
    }

    // Shutdown during the reconnect sleep ends the task without a new error
    shutdown.send(true).unwrap();
    task.await.unwrap();
    assert!(rx.recv().await.is_none(), "no events after shutdown");
}
