#![forbid(unsafe_code)]

//! `nopass-core`: pure logic for the NoPass sudoers-rule lifecycle.
//!
//! This crate has no I/O, no UI, and no system dependencies. It holds every
//! decision that can be made from bytes alone: rule rendering, header
//! grammar, expiry algebra, path layout, `login.defs` parsing, UTC
//! formatting, and the `HelperStatus` wire type.
//!
//! Phase 1 established the workspace skeleton and the baseline
//! `cargo test --workspace` gate. Phase 2 added the pure rule-rendering,
//! header-parsing, and expiry-algebra modules (`template`, `header`,
//! `expiry`). Phase 3 adds filesystem layout, the `HelperStatus` wire type,
//! `login.defs` admission-range parsing, and UTC formatting (`paths`,
//! `state`, `logindefs`, `timefmt`).

pub mod expiry;
pub mod header;
pub mod logindefs;
pub mod paths;
pub mod state;
pub mod template;
pub mod timefmt;
