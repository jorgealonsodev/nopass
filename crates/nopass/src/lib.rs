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
//!
//! Phase 5 adds `outcome` (the exit-code → outcome table, design.md §5,
//! §5.1) and `invoke` (`pkexec` argv construction and `ActionGate`,
//! design.md §4.3-4.4).
//!
//! Phase 6 adds `format` (icon-name resolution, design.md §7.1). Phase 7
//! completes `format` (countdown/tooltip/menu-label rendering, §7.2) and
//! adds `tray` (the only `ksni`-aware module, §2 `tray`).
//!
//! Phase 8 adds `notifications` (the only `notify-rust`-aware module,
//! §2 `notifications`, §7.3). Phase 9 adds `instance` (D-Bus name
//! ownership, `org.freedesktop.Application`, and the activation nudge,
//! §2 `instance`, §6.4, D3).
//!
//! Phase 10 adds `preflight` (the startup decision table and the polkit
//! readiness ladder, §0 G3, §8) and `app` (the single-owner reconciliation
//! loop, §2 `app`, §6), completes `event`'s `Event` enum, and wires
//! `main.rs`'s real startup sequence (§6.1).
//!
//! `m3-menu-and-config` Phase 1 adds `atomicfile` and `duration`. Phase 2
//! adds `config` — the `~/.config/nopass/config.toml` reader/writer
//! (design.md §4 D4). Phase 3 adds `consent` — the type that makes an
//! unconsented grant unrepresentable (design.md §3 D3). Phase 4 adds
//! `autostart` — the `~/.config/autostart/nopass.desktop` reader/writer
//! (design.md §4 D5).

pub mod atomicfile;
pub mod autostart;
pub mod config;
pub mod consent;
pub mod duration;
pub mod state;
pub mod runner;
pub mod probe;
pub mod reconcile;
pub mod event;
pub mod watch;
pub mod outcome;
pub mod invoke;
pub mod format;
pub mod tray;
pub mod notifications;
pub mod instance;
pub mod preflight;
pub mod app;
