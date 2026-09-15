//! `~/.config/nopass/config.toml`: the persisted default grant duration and
//! the "don't warn again" consent flag (design.md §4 D4; spec `user-config`
//! (all)).
//!
//! Mirrors `state.rs`'s `FileReading { Parsed, Absent, Faulted }` shape —
//! see this module's doc comments on [`ConfigReading`] for the one
//! deliberate asymmetry (design.md §4 D4: absence of `state.toml` is not
//! evidence about the world, but absence of `config.toml` legitimately
//! resolves to safe defaults, because every field of a user's preferences
//! has one).
//!
//! Phase 2 tasks 2.1–2.7 land here in order: `2.2` path resolution, `2.3`
//! `Absent` handling, `2.4` per-field tolerant parsing, `2.5`/`2.6` the
//! atomic write and its `.bak` rename of a faulted file.

use std::path::{Path, PathBuf};

use crate::atomicfile::{self, AtomicFileError};
use crate::duration::GrantDuration;

/// Every way [`read`] can fail to produce a usable [`Config`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFault {
    /// The file exists but could not be read (any I/O error other than
    /// "not found").
    Io,
    /// The bytes are not valid UTF-8, or do not parse as a TOML document
    /// at all (syntax error, truncated content, duplicate keys, etc.).
    Malformed,
}

/// What the config FILE says — and nothing more.
///
/// Deliberately asymmetric with `state.rs`'s `FileReading`: `state.rs`
/// forbids `From<FileReading> for TrayState` because the state file is a
/// claim about the *world*, where absence is not evidence. `config.toml`
/// is a claim about the *user's preferences*, where every field has a
/// safe, conservative default — so a caller MAY always turn a
/// [`ConfigReading`] into a usable [`Config`] via [`resolve`]. A malformed
/// config must never prevent the tray from starting: this file is
/// hand-edited by a user, and a tray that refuses to start over a stray
/// character in a preferences file is a tray the user cannot use to fix
/// the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigReading {
    Parsed(Config),
    Absent,
    Faulted(ConfigFault),
}

/// The two persisted preferences: the default grant duration a left-click
/// or an unqualified "Activate" offers, and whether the first-activation
/// consent warning has already been acknowledged.
///
/// `warning_acknowledged` is the in-memory sense (`true` = do not warn);
/// the persisted TOML key is `warn_before_activation` (the user-facing
/// sense) and is its exact negation on both read and write — see
/// [`read`]/[`write`]. The name and the negation are both intentional;
/// they must never drift into agreement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Config {
    pub default_duration: GrantDuration,
    pub warning_acknowledged: bool,
}

impl Config {
    /// The conservative schema defaults (spec `user-config` "Schema and
    /// Defaults"): `default_duration = 1h`, and re-warn until the user
    /// explicitly says otherwise (`warning_acknowledged = false`, i.e.
    /// persisted `warn_before_activation = true`).
    pub fn defaults() -> Self {
        Config { default_duration: GrantDuration::Hour1, warning_acknowledged: false }
    }
}

/// Resolves the config path: `$XDG_CONFIG_HOME/nopass/config.toml` when
/// `XDG_CONFIG_HOME` is set and non-empty, `~/.config/nopass/config.toml`
/// otherwise (spec `user-config` "Config Path Honors XDG_CONFIG_HOME").
///
/// Reads `std::env::var` directly. This is a plain read, not a mutation —
/// `std::env::set_var` is the one that requires `unsafe` under the 2024
/// edition and is forbidden by this crate's `#![forbid(unsafe_code)]` — so
/// production code goes through this real lookup, and [`path_from_env`] is
/// the seam tests use to exercise both branches deterministically without
/// touching the process environment at all (mirrors
/// `nopass-helper::uid::resolve` and `format::icon_name_for_style`).
pub fn path() -> PathBuf {
    path_from_env(std::env::var("XDG_CONFIG_HOME").ok().as_deref(), std::env::var("HOME").ok().as_deref())
}

/// [`path`]'s table, parameterised by explicit `XDG_CONFIG_HOME`/`HOME`
/// values rather than reading the environment — the seam this module's
/// own tests use to exercise both branches deterministically (spec
/// `user-config` "Config Path Honors XDG_CONFIG_HOME").
pub fn path_from_env(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    match xdg_config_home.filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir).join("nopass").join("config.toml"),
        None => {
            let home = home.filter(|v| !v.is_empty()).expect("HOME must be set to resolve the config path");
            PathBuf::from(home).join(".config").join("nopass").join("config.toml")
        }
    }
}

/// Reads and parses the config file at `path`.
///
/// `ENOENT` maps to [`ConfigReading::Absent`]. Every other I/O error maps
/// to `Faulted(ConfigFault::Io)`. Non-UTF-8 bytes and a document that does
/// not parse as TOML syntax at all (truncated content, duplicate keys,
/// stray `[[[`, ...) map to `Faulted(ConfigFault::Malformed)` — there is
/// nothing salvageable to preserve from bytes that aren't a document.
///
/// A document that DOES parse as valid TOML syntax always yields `Parsed`,
/// even when a field is missing, wrong-typed, or an unrecognized value: an
/// unknown key is ignored, and a bad `default_duration` falls back to
/// [`GrantDuration::Hour1`] without discarding a valid sibling
/// `warn_before_activation` in the same document (design.md §4 D4
/// "Per-field tolerance"; threat matrix "Config as foreign input" — an
/// unparsed `default_duration` never reaches anything but
/// [`GrantDuration::parse`]'s closed six-arm match).
pub fn read(path: &Path) -> ConfigReading {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return ConfigReading::Absent,
        Err(_) => return ConfigReading::Faulted(ConfigFault::Io),
    };

    let Ok(text) = std::str::from_utf8(&bytes) else {
        return ConfigReading::Faulted(ConfigFault::Malformed);
    };

    let Ok(doc) = text.parse::<toml_edit::DocumentMut>() else {
        return ConfigReading::Faulted(ConfigFault::Malformed);
    };

    let default_duration =
        doc.get("default_duration").and_then(toml_edit::Item::as_str).and_then(GrantDuration::parse);
    let default_duration = default_duration.unwrap_or(GrantDuration::Hour1);

    let warn_before_activation = doc.get("warn_before_activation").and_then(toml_edit::Item::as_bool).unwrap_or(true);

    ConfigReading::Parsed(Config { default_duration, warning_acknowledged: !warn_before_activation })
}

/// Turns a [`ConfigReading`] into a usable [`Config`], never refusing to
/// start: `Absent` and every `Faulted` arm resolve to [`Config::defaults`],
/// with the fault (when any) surfaced as the caller's cue to warn — never
/// as a reason to stop (spec `user-config` "Tolerant Parsing Never Blocks
/// Startup"; design.md §4 D4).
pub fn resolve(reading: &ConfigReading) -> (Config, Option<ConfigFault>) {
    match reading {
        ConfigReading::Parsed(config) => (*config, None),
        ConfigReading::Absent => (Config::defaults(), None),
        ConfigReading::Faulted(fault) => (Config::defaults(), Some(*fault)),
    }
}

/// Writes `config` to `path` atomically, preserving any comments and
/// unknown keys already present in the file at `path` — a config writer
/// that reformats a hand-edited file loses the operator's own notes.
///
/// Reads whatever currently exists at `path` first, to decide how to
/// build the [`toml_edit::DocumentMut`] the two known keys are set on:
///
/// - Absent, or any I/O error reading it: nothing to preserve or back up,
///   start from a fresh empty document.
/// - Present but not valid UTF-8/TOML (a faulted reading): the user's
///   broken hand-edit is preserved, not destroyed — it is renamed to
///   `<file>.bak` (best-effort; a failure here does not abort the write)
///   before a fresh document is built (design.md §4 D4).
/// - Present and valid TOML: that exact document is reused, so every
///   comment, blank line, and unknown key survives — only
///   `default_duration` and `warn_before_activation` are ever set.
///
/// The write itself goes through [`atomicfile::write`] with mode `0600` —
/// this crate never invents a second atomic-write path.
pub fn write(path: &Path, config: &Config) -> Result<(), AtomicFileError> {
    let mut doc = match std::fs::read(path) {
        Ok(bytes) => match std::str::from_utf8(&bytes).ok().and_then(|text| text.parse::<toml_edit::DocumentMut>().ok())
        {
            Some(existing) => existing,
            None => {
                let bak_path = backup_path_for(path);
                let _ = std::fs::rename(path, &bak_path);
                toml_edit::DocumentMut::new()
            }
        },
        Err(_) => toml_edit::DocumentMut::new(),
    };

    doc["default_duration"] = toml_edit::value(config.default_duration.config_key());
    doc["warn_before_activation"] = toml_edit::value(!config.warning_acknowledged);

    atomicfile::write(path, doc.to_string().as_bytes(), 0o600)
}

/// The `.bak` sibling [`write`] renames a faulted existing file to before
/// overwriting it (task 2.6): `<dir>/<file-name>.bak`, e.g.
/// `config.toml` → `config.toml.bak`.
fn backup_path_for(path: &Path) -> PathBuf {
    let file_name = path.file_name().expect("config::write given a path with no file name").to_string_lossy();
    path.with_file_name(format!("{file_name}.bak"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- path resolution (task 2.2) ----

    #[test]
    fn xdg_config_home_overrides_the_default_path_when_set_and_non_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let xdg = tmp.path().join("custom-xdg");
        let xdg_str = xdg.to_str().unwrap();

        let resolved = path_from_env(Some(xdg_str), Some("/home/someone"));

        assert_eq!(resolved, xdg.join("nopass").join("config.toml"));
    }

    #[test]
    fn default_path_is_used_when_xdg_config_home_is_unset() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home-dir");
        let home_str = home.to_str().unwrap();

        let resolved = path_from_env(None, Some(home_str));

        assert_eq!(resolved, home.join(".config").join("nopass").join("config.toml"));
    }

    #[test]
    fn empty_xdg_config_home_falls_back_to_the_home_based_default_path() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home-dir");
        let home_str = home.to_str().unwrap();

        let resolved = path_from_env(Some(""), Some(home_str));

        assert_eq!(resolved, home.join(".config").join("nopass").join("config.toml"));
    }

    // ---- Absent handling in resolve (task 2.3) ----

    #[test]
    fn resolve_of_absent_yields_schema_defaults_and_no_fault() {
        let (config, fault) = resolve(&ConfigReading::Absent);

        assert_eq!(config.default_duration, GrantDuration::Hour1);
        assert!(!config.warning_acknowledged, "warn_before_activation defaults to true, i.e. warning_acknowledged=false");
        assert_eq!(fault, None);
    }

    #[test]
    fn resolve_of_faulted_yields_schema_defaults_and_surfaces_the_fault() {
        let (config, fault) = resolve(&ConfigReading::Faulted(ConfigFault::Malformed));

        assert_eq!(config, Config::defaults());
        assert_eq!(fault, Some(ConfigFault::Malformed));
    }

    #[test]
    fn resolve_of_parsed_passes_the_config_through_unchanged_with_no_fault() {
        let stored = Config { default_duration: GrantDuration::Hours8, warning_acknowledged: true };

        let (config, fault) = resolve(&ConfigReading::Parsed(stored));

        assert_eq!(config, stored);
        assert_eq!(fault, None);
    }
}
