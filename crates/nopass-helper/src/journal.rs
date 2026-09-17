//! journald audit trail (design.md §9; helper-observability §Journald
//! Audit Records).
//!
//! Two responsibilities: [`init`] configures the global `tracing`
//! subscriber once, at process start, and [`audit`] emits one structured
//! record per `enable`/`disable`/`expire` outcome.
//!
//! **Why the audit field names are NOT already prefixed `NOPASS_` at the
//! call site.** `tracing_journald::Layer::with_field_prefix` (configured
//! in [`init`] below, per design.md §9's literal code block) prepends
//! its configured prefix — `"NOPASS"` here — to every user-defined field
//! name automatically, once, at the journald-transport layer. If [`audit`]
//! *also* named its fields `NOPASS_EVENT`/`NOPASS_UID`/etc., a record
//! routed through the journald layer would come out double-prefixed
//! (`NOPASS_NOPASS_EVENT`). So [`audit`] emits the bare, unprefixed names
//! design.md §9 lists the *values* for (`EVENT`, `UID`, `USER`,
//! `OUTCOME`, `EXPIRES`, `EXIT`, `REASON`), and the journald layer's own
//! `with_field_prefix` is solely responsible for turning `EVENT` into
//! `NOPASS_EVENT` on that specific transport. The stderr fallback layer
//! has no equivalent prefixing mechanism and therefore prints the bare
//! names — acceptable, since design.md only requires the `NOPASS_`
//! prefix for the journald audit trail itself, not for the
//! no-journald-available fallback.

use nopass_core::expiry::Expiry;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::subject::UidSource;

/// `tracing_journald`'s syslog identifier for every record this helper
/// emits (design.md §9, `SYSLOG_IDENTIFIER=nopass-helper`).
pub const SYSLOG_IDENTIFIER: &str = "nopass-helper";
/// `tracing_journald`'s field prefix — see the module doc comment for
/// why [`audit`]'s own field names must NOT also carry this prefix.
pub const FIELD_PREFIX: &str = "NOPASS";

/// Opt-out env var checked ONLY by [`init`], never by [`audit`] itself —
/// a real invocation (`pkexec`/systemd, both of which hand this binary a
/// controlled/cleared environment; see `main.rs`'s `run()`) never sets
/// it, so the production path is unaffected by construction. It exists
/// for integration tests that spawn the REAL compiled `nopass-helper`
/// binary (`tests/process_boundary.rs`) to exercise unrelated
/// process-boundary behaviour (exit codes, stdout shape) on an
/// unprivileged host that genuinely has a live journald socket: without
/// this, every such spawn is a fresh process that calls the real
/// `init()` and, if it reaches an audited outcome, writes a REAL,
/// fabricated record into the HOST's real audit trail as an unrelated
/// side effect of testing something else entirely. Tests that
/// deliberately want the real journald write-and-read-back path
/// (`tests/root_journal.rs`'s own container lane) must never set this.
///
/// **Never reachable in a release build.** This constant — and the
/// `init()` branch that checks it — is compiled only `#[cfg(debug_
/// assertions)]`: `cargo build --release`/`cargo deb` both build with
/// `debug-assertions = false` (the workspace's `[profile.release]`
/// carries no override, so that is cargo's own default), and every test
/// lane that needs this opt-out (`process_boundary.rs`'s own doc
/// comment, this module's own `#[cfg(test)]` tests) runs through `cargo
/// test`, which always builds the `test`/`dev` profile — `debug-
/// assertions = true` — regardless of which lane script invokes it (see
/// `scripts/run-lane-root.sh`, `run-lane-journal.sh`, `run-lane-polkit.sh`,
/// all of which run `cargo test`, never `cargo build --release`). The
/// one Containerfile that DOES build `--release`
/// (`tests/containers/Containerfile.systemd`) is explicitly documented
/// as "NOT part of `cargo test --workspace` and never run by CI" — a
/// manual, human-observed lane, not a gate this opt-out needs to survive
/// in. Without the `#[cfg]`, a root-owning operator could run
/// `sudo NOPASS_HELPER_TEST_DISABLE_JOURNALD=1 nopass-helper grant
/// --uid N --until-reboot` against the shipped binary and grant
/// passwordless sudo with no audit record at all — not a privilege
/// escalation (they already had sudo), but a silent erasure of the one
/// record that exists to prove it happened.
#[cfg(debug_assertions)]
pub const DISABLE_JOURNALD_ENV: &str = "NOPASS_HELPER_TEST_DISABLE_JOURNALD";

/// Configures the global `tracing` subscriber: a `tracing-journald`
/// layer when the journald socket is reachable, otherwise a stderr `fmt`
/// layer (design.md §9 — "If the journald socket is unavailable the
/// helper falls back to the stderr layer and continues — logging never
/// fails an operation"). Uses `try_init` rather than the plain `init`
/// design.md's code block shows so a second call (a defensive re-init,
/// or multiple tests in the same process) is a harmless no-op instead of
/// a panic; the configuration is otherwise identical.
pub fn init() {
    // `#[cfg(debug_assertions)]`: this opt-out — constant AND check —
    // does not exist at all in a release build. See
    // `DISABLE_JOURNALD_ENV`'s own doc comment for why that cfg is safe
    // for every test lane that needs it, and why it must never be
    // reachable from the shipped binary.
    #[cfg(debug_assertions)]
    if std::env::var(DISABLE_JOURNALD_ENV).is_ok() {
        // An explicit, test-only opt-out, never reachable from a real
        // pkexec/systemd invocation. Falls back to the exact same
        // stderr layer `init` already uses when no journald socket is
        // reachable at all — logging still never fails an operation,
        // it is just never routed to the REAL host journal for this
        // one process.
        let _ = tracing_subscriber::registry()
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
            .try_init();
        return;
    }
    match tracing_journald::layer() {
        Ok(layer) => {
            let _ = tracing_subscriber::registry()
                .with(
                    layer
                        .with_syslog_identifier(SYSLOG_IDENTIFIER.to_string())
                        .with_field_prefix(Some(FIELD_PREFIX.to_string())),
                )
                .try_init();
        }
        Err(_) => {
            let _ = tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
                .try_init();
        }
    }
}

/// Which transaction produced an audit record (`NOPASS_EVENT`). The
/// modified helper-observability "Journald Audit Records" requirement
/// binds by "every outcome-producing subcommand", not by an enumerated
/// name list, precisely so a future subcommand cannot ship unaudited by
/// omission — `audit_event_for` below is the exhaustive, wildcard-free
/// map that enforces this at compile time. `Status` has been part of the
/// documented value domain since M1; `ops::status` (design.md §4.5) now
/// emits it on every successful call, closing the gap where it used to
/// be emitted from nowhere. `Grant`, `Revoke`, and `Inspect` are the
/// three headless root-context subcommands `m3a-headless-grant` adds
/// (design.md §1); Phase 5 gave them their `Cmd` variants and Phase 7
/// their `journal::audit` call sites, so `audit_event_for` reaches all
/// seven today — see the totality test at the bottom of this file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    Enable,
    Disable,
    Expire,
    Status,
    Grant,
    Revoke,
    Inspect,
}

impl AuditEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditEvent::Enable => "enable",
            AuditEvent::Disable => "disable",
            AuditEvent::Expire => "expire",
            AuditEvent::Status => "status",
            AuditEvent::Grant => "grant",
            AuditEvent::Revoke => "revoke",
            AuditEvent::Inspect => "inspect",
        }
    }
}

/// Total, wildcard-free map from every `Cmd` variant to the `AuditEvent`
/// it is audited under (design.md §4 "Growth that closes the omission
/// hole"; threat matrix "Unaudited new subcommand"). No wildcard arm
/// means a subcommand added to `Cmd` without a corresponding arm here
/// fails to compile, rather than silently shipping unaudited — exactly
/// what happened here: `m3a-headless-grant` Phase 5 added `Grant`,
/// `Revoke`, and `Inspect` to `Cmd`, which broke this match until the
/// three arms below were added. Over the now-seven-variant `Cmd` surface,
/// this map is total by construction and injective into the
/// now-seven-variant `AuditEvent` domain.
pub fn audit_event_for(cmd: &crate::cli::Cmd) -> AuditEvent {
    match cmd {
        crate::cli::Cmd::Enable { .. } => AuditEvent::Enable,
        crate::cli::Cmd::Disable => AuditEvent::Disable,
        crate::cli::Cmd::Status => AuditEvent::Status,
        crate::cli::Cmd::Expire { .. } => AuditEvent::Expire,
        crate::cli::Cmd::Grant { .. } => AuditEvent::Grant,
        crate::cli::Cmd::Revoke { .. } => AuditEvent::Revoke,
        crate::cli::Cmd::Inspect { .. } => AuditEvent::Inspect,
    }
}

/// The outcome of one audited transaction (`NOPASS_OUTCOME`), per
/// design.md §9's exact documented value domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditOutcome {
    Ok,
    Rejected,
    SkippedNotExpired,
    RolledBack,
    Error,
}

impl AuditOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            AuditOutcome::Ok => "ok",
            AuditOutcome::Rejected => "rejected",
            AuditOutcome::SkippedNotExpired => "skipped_not_expired",
            AuditOutcome::RolledBack => "rolled_back",
            AuditOutcome::Error => "error",
        }
    }
}

/// Renders `NOPASS_EXPIRES`'s three-value domain (`never|reboot|<epoch>`,
/// design.md §9). There is no "not applicable" value in that domain, so
/// callers auditing an outcome with no meaningful expiry (e.g. an
/// `enable` that never reached the point of establishing one) pass
/// `Expiry::Never` as the closest honest sentinel.
fn expires_field(expires: Expiry) -> String {
    match expires {
        Expiry::Never => "never".to_string(),
        Expiry::Reboot => "reboot".to_string(),
        Expiry::At { epoch } => epoch.to_string(),
    }
}

/// One audit record's full field set. `reason` is a short stable token
/// (design.md §9: "never raw subprocess text") — `sudo`/`visudo` stderr
/// is captured into the log only via `tracing::error!`/`debug!` call
/// sites elsewhere, never through this struct.
///
/// `context` (design.md §4 "Audit schema growth — one new field") is
/// read straight through by [`audit`] below and MUST be populated from
/// `subject.source()` at every call site, NEVER derived from `event` —
/// the subcommand name stopped implying the invocation context the
/// moment `grant` and `enable` could both produce a rule for the same
/// uid. There is no fallback path in [`audit`] that could paper over a
/// caller getting this wrong.
pub struct AuditRecord<'a> {
    pub event: AuditEvent,
    pub context: UidSource,
    pub uid: u32,
    pub user: &'a str,
    pub outcome: AuditOutcome,
    pub expires: Expiry,
    pub exit: i32,
    pub reason: &'a str,
}

/// Emits one journald/stderr record for `record` with exactly the eight
/// fields design.md §4/§9 document (before the journald layer's own
/// `NOPASS_` prefixing — see the module doc comment): `EVENT`, `UID`,
/// `USER`, `OUTCOME`, `EXPIRES`, `EXIT`, `REASON`, `CONTEXT`. `MESSAGE`
/// and `SYSLOG_IDENTIFIER` are supplied by the `tracing`/`tracing-journald`
/// machinery itself, not by this function. `CONTEXT` is read straight
/// through from `record.context` — a plain field read, no match on
/// `record.event` anywhere in this function.
pub fn audit(record: &AuditRecord) {
    let expires = expires_field(record.expires);
    tracing::info!(
        EVENT = record.event.as_str(),
        UID = record.uid,
        USER = record.user,
        OUTCOME = record.outcome.as_str(),
        EXPIRES = expires.as_str(),
        EXIT = record.exit,
        REASON = record.reason,
        CONTEXT = record.context.as_str(),
        "nopass audit event"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct SharedBuf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedBuf {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedBuf {
        type Writer = SharedBuf;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn syslog_identifier_and_field_prefix_are_pinned_per_design_md_section_9() {
        assert_eq!(SYSLOG_IDENTIFIER, "nopass-helper");
        assert_eq!(FIELD_PREFIX, "NOPASS");
    }

    /// Env flag selecting the re-exec'd CHILD branch below — same
    /// process-isolation idiom `runner.rs`'s
    /// `system_runner_clears_ambient_env_and_keeps_pkexec_uid_out_of_the_child`
    /// and `fileops_tempdir.rs`'s `mode_0440_holds_even_under_umask_0o077`
    /// already use for a test whose real production call has a
    /// process-global side effect that must never bleed into any other
    /// test sharing this binary's process.
    const INIT_TEST_CHILD_FLAG: &str = "NOPASS_HELPER_INIT_TEST_CHILD";

    #[test]
    fn init_does_not_panic_in_a_sandboxed_environment_with_no_journald_socket() {
        // `init()` calls `tracing_subscriber::registry()...try_init()`,
        // which — unlike a scoped `tracing::subscriber::with_default` —
        // installs a REAL, process-GLOBAL default dispatcher exactly
        // once per process (later calls are a harmless no-op, which is
        // this test's actual assertion: "does not panic"). On a CI
        // sandbox with no journald socket that global default is the
        // inert stderr fallback layer, so calling `init()` in-process
        // used to look harmless. On a real desktop with a live journald
        // socket (this box, among others) `init()` instead installs the
        // REAL `tracing_journald` layer as that same process-global
        // default — and every OTHER test in this binary that calls an
        // audited `ops::*` path without its own scoped subscriber (most
        // of them do not assert on the audit trail, so most of them
        // never bothered to scope one) then silently inherits it,
        // writing fabricated `grant`/`revoke`/`enable`/… records into
        // the REAL host journal. That is a real, measured bug (fixed
        // here), not a hypothetical one.
        //
        // The fix is process isolation, not weakening `init()`: this
        // test still calls the real, unmodified `init()` and still
        // proves it never panics on either branch, but it does so in a
        // throwaway re-exec'd CHILD process (`current_exe()` + `--exact`
        // + this one test), so whichever global default `init()`
        // actually installs — stderr layer or real journald layer — is
        // scoped to that child's own short-lived process and can never
        // become the ambient default any other test in THIS process
        // inherits.
        if std::env::var(INIT_TEST_CHILD_FLAG).is_ok() {
            init();
            return;
        }

        let exe = std::env::current_exe().expect("current test binary path");
        let output = std::process::Command::new(exe)
            .arg("journal::tests::init_does_not_panic_in_a_sandboxed_environment_with_no_journald_socket")
            .arg("--exact")
            .arg("--nocapture")
            .env(INIT_TEST_CHILD_FLAG, "1")
            .output()
            .expect("spawn init()-isolated child test process");
        assert!(
            output.status.success(),
            "init()-isolated child test failed (status {:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    /// Env flag selecting the re-exec'd CHILD branch below — separate
    /// from `INIT_TEST_CHILD_FLAG` above so this test's own
    /// `DISABLE_JOURNALD_ENV=1` (the thing under test) can never be
    /// confused with the outer/inner selector.
    const DISABLE_JOURNALD_TEST_CHILD_FLAG: &str = "NOPASS_HELPER_DISABLE_JOURNALD_TEST_CHILD";

    /// Fix 1 regression pin: `init()` with `DISABLE_JOURNALD_ENV` set
    /// must take the stderr-fallback branch and never panic, exactly
    /// like the no-socket case above — proving the opt-out
    /// `tests/process_boundary.rs` now sets on every real-binary spawn
    /// actually short-circuits `init()` before it ever probes (let
    /// alone binds) the real journald socket. Isolated into its own
    /// re-exec'd child process for the same reason as the test above:
    /// `init()`'s `try_init()` is a real, process-global, one-shot side
    /// effect that must never leak into any other test sharing this
    /// binary's process.
    #[test]
    fn init_with_the_disable_journald_env_set_never_panics_and_skips_the_real_socket() {
        if std::env::var(DISABLE_JOURNALD_TEST_CHILD_FLAG).is_ok() {
            // SAFETY (not unsafe, just a note): this is the same
            // `Command::env`-only pattern the outer branch below (and
            // `runner.rs`'s adversarial-env test) already uses — the
            // env var is set in THIS re-exec'd child's own environment
            // by the parent's `Command::env` call, never via
            // `std::env::set_var` in-process.
            assert!(
                std::env::var(DISABLE_JOURNALD_ENV).is_ok(),
                "the child process must have inherited {DISABLE_JOURNALD_ENV}=1 from the parent's Command::env"
            );
            init();
            return;
        }

        let exe = std::env::current_exe().expect("current test binary path");
        let output = std::process::Command::new(exe)
            .arg("journal::tests::init_with_the_disable_journald_env_set_never_panics_and_skips_the_real_socket")
            .arg("--exact")
            .arg("--nocapture")
            .env(DISABLE_JOURNALD_TEST_CHILD_FLAG, "1")
            .env(DISABLE_JOURNALD_ENV, "1")
            .output()
            .expect("spawn DISABLE_JOURNALD_ENV-isolated child test process");
        assert!(
            output.status.success(),
            "DISABLE_JOURNALD_ENV-isolated child test failed (status {:?}):\nstdout: {}\nstderr: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[test]
    fn audit_emits_every_documented_field_name_and_value() {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));

        tracing::subscriber::with_default(subscriber, || {
            audit(&AuditRecord {
                event: AuditEvent::Enable,
                context: UidSource::Pkexec,
                uid: 1000,
                user: "jorge",
                outcome: AuditOutcome::Ok,
                expires: Expiry::At { epoch: 1_789_000_000 },
                exit: 0,
                reason: "",
            });
        });

        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        // `tracing_subscriber::fmt`'s default formatter quotes string
        // field values (`EVENT="enable"`) but not numeric ones
        // (`UID=1000`) — this is a formatting detail of the STDERR
        // fallback layer's text renderer; the actual journald transport
        // (`tracing_journald::Layer`) encodes fields with a fixed binary
        // framing, not this text format, and applies its own
        // `with_field_prefix`, per the module doc comment.
        for expected in [
            "EVENT=\"enable\"",
            "UID=1000",
            "USER=\"jorge\"",
            "OUTCOME=\"ok\"",
            "EXPIRES=\"1789000000\"",
            "EXIT=0",
            "CONTEXT=\"Pkexec\"",
        ] {
            assert!(text.contains(expected), "expected {expected:?} in captured audit output: {text}");
        }
    }

    #[test]
    fn audit_reports_the_never_and_reboot_expiry_sentinels_and_a_nonzero_exit() {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));

        tracing::subscriber::with_default(subscriber, || {
            audit(&AuditRecord {
                event: AuditEvent::Disable,
                context: UidSource::Pkexec,
                uid: 2000,
                user: "ana",
                outcome: AuditOutcome::Error,
                expires: Expiry::Never,
                exit: 16,
                reason: "fs_error",
            });
            audit(&AuditRecord {
                event: AuditEvent::Expire,
                context: UidSource::SystemRoot,
                uid: 3000,
                user: "sam",
                outcome: AuditOutcome::SkippedNotExpired,
                expires: Expiry::Reboot,
                exit: 0,
                reason: "",
            });
        });

        let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        assert!(text.contains("EVENT=\"disable\""));
        assert!(text.contains("OUTCOME=\"error\""));
        assert!(text.contains("EXPIRES=\"never\""));
        assert!(text.contains("EXIT=16"));
        assert!(text.contains("REASON=\"fs_error\""));
        assert!(text.contains("CONTEXT=\"Pkexec\""));
        assert!(text.contains("EVENT=\"expire\""));
        assert!(text.contains("OUTCOME=\"skipped_not_expired\""));
        assert!(text.contains("EXPIRES=\"reboot\""));
        assert!(text.contains("CONTEXT=\"SystemRoot\""));
    }

    /// Shared capture helper for the task 4.2/4.3 tests below — same
    /// technique as the two tests above, factored out because both new
    /// tests emit several records and inspect the combined text.
    fn capture<F: FnOnce()>(f: F) -> String {
        let buf = SharedBuf::default();
        let subscriber =
            tracing_subscriber::registry().with(tracing_subscriber::fmt::layer().with_writer(buf.clone()).with_ansi(false));
        tracing::subscriber::with_default(subscriber, f);
        String::from_utf8(buf.0.lock().unwrap().clone()).unwrap()
    }

    // --- task 4.2: a Pkexec-context record and a SystemRoot-context
    // record for the SAME uid are distinguishable on CONTEXT alone
    // (helper-observability "A root-invoked grant is journaled with the
    // SystemRoot context and its explicit target"; "Revoke and inspect
    // are journaled the same way") ---------------------------------------

    #[test]
    fn pkexec_and_system_root_context_records_for_the_same_uid_are_distinguishable_on_context_alone() {
        let text = capture(|| {
            audit(&AuditRecord {
                event: AuditEvent::Enable,
                context: UidSource::Pkexec,
                uid: 1000,
                user: "jorge",
                outcome: AuditOutcome::Ok,
                expires: Expiry::Never,
                exit: 0,
                reason: "",
            });
            audit(&AuditRecord {
                event: AuditEvent::Grant,
                context: UidSource::SystemRoot,
                uid: 1000,
                user: "jorge",
                outcome: AuditOutcome::Ok,
                expires: Expiry::Never,
                exit: 0,
                reason: "",
            });
        });

        assert!(text.contains("CONTEXT=\"Pkexec\""), "expected a Pkexec-context record for uid 1000: {text}");
        assert!(text.contains("CONTEXT=\"SystemRoot\""), "expected a SystemRoot-context record for uid 1000: {text}");
    }

    // --- task 4.3: the non-negotiable test. `context` is read exclusively
    // from `record.context` (which production code populates from
    // `subject.source()`), NEVER inferred from `record.event` — a
    // deliberately mismatched event/context pairing must still emit
    // exactly the `context` value given, proving `audit()` has no
    // event-based fallback or override anywhere. Covers pairings in BOTH
    // directions (an event that would normally be `SystemRoot` paired
    // with `Pkexec`, and vice versa) so a partial/event-keyed derivation
    // cannot slip through by only checking one direction. ------------------

    #[test]
    fn context_is_read_from_the_record_never_inferred_from_its_event() {
        let mismatched = [
            // Grant/Revoke/Inspect/Expire are SystemRoot-context in every
            // real invocation — deliberately paired with Pkexec here.
            (AuditEvent::Grant, UidSource::Pkexec),
            (AuditEvent::Revoke, UidSource::Pkexec),
            (AuditEvent::Inspect, UidSource::Pkexec),
            (AuditEvent::Expire, UidSource::Pkexec),
            // Enable/Disable/Status are Pkexec-context in every real
            // invocation — deliberately paired with SystemRoot here.
            (AuditEvent::Enable, UidSource::SystemRoot),
            (AuditEvent::Disable, UidSource::SystemRoot),
            (AuditEvent::Status, UidSource::SystemRoot),
        ];

        for (event, context) in mismatched {
            let text = capture(|| {
                audit(&AuditRecord {
                    event,
                    context,
                    uid: 5000,
                    user: "probe",
                    outcome: AuditOutcome::Ok,
                    expires: Expiry::Never,
                    exit: 0,
                    reason: "",
                });
            });
            let expected = format!("CONTEXT=\"{}\"", context.as_str());
            assert!(
                text.contains(&expected),
                "event {event:?} paired with context {context:?} must still emit {expected:?} — \
                 CONTEXT must never be derived from EVENT. Captured: {text}"
            );
        }
    }

    // --- task 2.2: new AuditEvent variants' as_str() matches the literal
    // token the spec names (helper-observability "Journald Audit
    // Records") ---------------------------------------------------------

    #[test]
    fn grant_revoke_inspect_as_str_match_the_spec_literal_tokens() {
        assert_eq!(AuditEvent::Grant.as_str(), "grant");
        assert_eq!(AuditEvent::Revoke.as_str(), "revoke");
        assert_eq!(AuditEvent::Inspect.as_str(), "inspect");
    }

    // --- task 2.5: `audit_event_for` is total (every `Cmd` variant maps)
    // and injective (no two `Cmd` variants map to the same `AuditEvent`)
    // over the current `Cmd` surface, mapped into the now-seven-variant
    // `AuditEvent` domain (design.md §4 "Growth that closes the omission
    // hole") -------------------------------------------------------------

    #[test]
    fn audit_event_for_is_total_and_injective_over_the_current_cmd_surface() {
        let cmds = [
            crate::cli::Cmd::Enable { until: None, until_reboot: false },
            crate::cli::Cmd::Disable,
            crate::cli::Cmd::Status,
            crate::cli::Cmd::Expire { uid: Some(1000), boot: false },
            crate::cli::Cmd::Grant { uid: 1000, until: None, until_reboot: false },
            crate::cli::Cmd::Revoke { uid: 1000 },
            crate::cli::Cmd::Inspect { uid: 1000 },
        ];
        let mapped: Vec<AuditEvent> = cmds.iter().map(audit_event_for).collect();
        assert_eq!(
            mapped,
            vec![
                AuditEvent::Enable,
                AuditEvent::Disable,
                AuditEvent::Status,
                AuditEvent::Expire,
                AuditEvent::Grant,
                AuditEvent::Revoke,
                AuditEvent::Inspect,
            ]
        );

        // Injective: no two distinct mapped events collide.
        for (i, a) in mapped.iter().enumerate() {
            for (j, b) in mapped.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "audit_event_for must be injective, but index {i} and {j} collided");
                }
            }
        }
    }
}
