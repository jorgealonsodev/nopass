#![forbid(unsafe_code)]

//! `nopass` library surface — the tray application.
//!
//! Mirrors `nopass-helper`'s thin-binary-over-library split: `src/main.rs`
//! is the real process entry point, and this crate root owns every module
//! so that `crates/nopass/tests/*.rs` (genuine Cargo integration tests,
//! each compiled as its own crate) can reach them through the public
//! library API rather than a private binary-only module tree.
//!
//! Phase 2 adds `state` — the reader whose return type makes "inactive
//! because the file was missing" unrepresentable (design.md §3.1, D3).
//! `probe`, `reconcile`, `outcome`, `format`, `runner`, `invoke`, `watch`,
//! `tray`, `notifications`, `instance`, `preflight`, `event` and `app`
//! each land in the phase that first needs them (design.md §1, §2).
//!
//! Phase 4 adds `watch` (the inotify directory watch, D8) and a partial
//! `event` (the `Event` enum's watch/probe-producible variants only —
//! see `event.rs` for why the rest waits for Phase 10).

pub mod state;
pub mod runner;
pub mod probe;
pub mod reconcile;
pub mod event;
pub mod watch;
