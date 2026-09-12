#![forbid(unsafe_code)]

//! `nopass` library surface — the tray application.
//!
//! Mirrors `nopass-helper`'s thin-binary-over-library split: `src/main.rs`
//! is the real process entry point, and this crate root owns every module
//! so that `crates/nopass/tests/*.rs` (genuine Cargo integration tests,
//! each compiled as its own crate) can reach them through the public
//! library API rather than a private binary-only module tree.
//!
//! Phase 1 (this file) declares no modules yet — `state`, `probe`,
//! `reconcile`, `outcome`, `format`, `runner`, `invoke`, `watch`, `tray`,
//! `notifications`, `instance`, `preflight`, `event` and `app` each land
//! in the phase that first needs them (design.md §1, §2).
