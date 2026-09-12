#![forbid(unsafe_code)]

//! `nopass`: the tray application, a thin entry point over the
//! `nopass` library (`src/lib.rs`), which owns every module — see
//! `lib.rs` for why the library exists.
//!
//! Phase 1 ships only this stub: `boot()` is a placeholder that will be
//! replaced by the real startup sequence (session-bus connection,
//! preflight, SNI registration, inotify watch, 60 s tick — design.md §6.1
//! steps 1–11) in Phase 10, task 10.7.

fn main() {
    std::process::exit(boot())
}

/// Phase 1 placeholder for the real startup sequence built in Phase 10.
fn boot() -> i32 {
    0
}
