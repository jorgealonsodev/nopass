#![forbid(unsafe_code)]

//! `nopass-helper` library surface.
//!
//! This crate builds both a binary (`src/main.rs`, the real privileged
//! entry point) and this library. The library exists so that
//! `tests/fileops_tempdir.rs` — a genuine Cargo integration test, compiled
//! as its own crate — can exercise `fileops`/`lock`/`bins`/`runner`/
//! `error` against `Layout::under(TempDir)` without spawning the real
//! privileged binary. A private (`mod`-only, no `lib.rs`) binary crate
//! cannot be depended on by anything outside itself, including its own
//! `tests/` directory (Cargo integration tests: are always a separate
//! crate that can only see a package's PUBLIC library API), so `fileops`
//! could not otherwise be reached from `tests/fileops_tempdir.rs` at all.
//! This mirrors the well-established Rust "thin binary over a library"
//! pattern; `main.rs` now just calls into `nopass_helper::run()`
//! equivalents via the re-exported modules below rather than declaring
//! its own private module tree, but its own behavior (`dispatch`, exit
//! code mapping) is unchanged.
//!
//! Phase 6 adds `lock` (the `flock` mutation-serialization guard) and
//! `fileops` (atomic sudoers-rule file operations). Neither is wired into
//! `main`'s `dispatch` yet — that lands with the real
//! `ops::{enable,disable,status,expire}` transactions in Phase 7.

pub mod bins;
pub mod checks;
pub mod cli;
pub mod error;
pub mod fileops;
pub mod lock;
pub mod runner;
pub mod uid;
