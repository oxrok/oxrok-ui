//! Oxrok UI: a Rust/egui logic analyzer, built on
//! [oxrok](https://github.com/oxrok/oxrok).
//!
//! The modules, bottom-up: `core` (time, formatting, view state),
//! `store` (sample storage), `backend` (the device backend contract),
//! `file` (`.oxrok` captures, import and export), `engine` (measurements,
//! triggers, navigation), `decode` (protocol decoders), `oxrok` (the
//! oxrok backend; the `oxrok` feature), `ui` (egui widgets and views),
//! `app` (the application shell) and `hil` (hardware test support).

pub mod app;
pub mod backend;
pub mod core;
pub mod decode;
pub mod engine;
pub mod file;
pub mod hil;
pub mod oxrok;
pub mod store;
pub mod ui;
