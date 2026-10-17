//! tuigps-rs library crate. The binary in `main.rs` is a thin wrapper so
//! integration tests under `tests/` can exercise these modules directly.

pub mod app;
pub mod clock_sync;
pub mod config;
pub mod constants;
pub mod data_model;
pub mod formatting;
pub mod gps_logger;
pub mod gpsd_client;
pub mod position_hold;
pub mod ui;
