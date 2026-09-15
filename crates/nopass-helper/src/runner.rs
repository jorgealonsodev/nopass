//! `CommandRunner` port: every external process spawn goes through this
//! trait so exact argv can be pinned in tests without touching the real
//! binaries (design.md §3; threat matrix "External command composition").
//!
//! `checks`, `timer`, and `fileops` (Phases 5-7) are the modules that
//! actually invoke a `CommandRunner` in production; `ops.rs` (Phase 7) is
//! the only caller wired into `main`'s `dispatch` so far. No
//! `#[allow(dead_code)]` is needed despite that: every item below is
//! `pub` inside this crate's `pub mod runner` (declared in `lib.rs`),
//! which makes it public library API exempt from the `dead_code` lint.

#[cfg(test)]
use std::cell::RefCell;
#[cfg(test)]
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;

/// One external command invocation: absolute program path, argv, and the
/// minimal environment it is allowed to see. Never a shell string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Applied after `env_clear()` — this is the *entire* child
    /// environment, nothing is inherited.
    pub env: Vec<(String, String)>,
    pub expect: Expect,
}

/// Whether the caller requires a zero exit status, or inspects the status
/// itself (used by `systemctl stop`, which tolerates a non-loaded unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Zero,
    Any,
}

/// The result of running a `CommandSpec`. `status: None` never appears in
/// a `SystemRunner` outcome — see [`RunnerError::Signaled`] — but stays
/// `Option<i32>` so a `ScriptedRunner` script can still express it for
/// exhaustiveness at call sites built in later phases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunnerError {
    /// The spec's `program` was not an absolute path. `env_clear()` alone
    /// does not guarantee no `PATH`-style lookup: glibc's `execvp` falls
    /// back to `confstr(_CS_PATH)` when `PATH` is entirely absent from
    /// the child env, so a bare or relative program name can still
    /// resolve and run. Every caller is expected to route the program
    /// through `Binaries::resolve` (design.md §3), but this is a
    /// privileged binary, so the guarantee is enforced structurally here
    /// rather than left to caller convention. Returned BEFORE anything
    /// is spawned.
    #[error("program path is not absolute: {program}")]
    NonAbsoluteProgram { program: String },
    #[error("failed to spawn {program}: {reason}")]
    Spawn { program: String, reason: String },
    /// `status: None` — the child died by signal. Always a failure, never
    /// a success, regardless of `Expect`.
    #[error("{program} was killed by a signal")]
    Signaled { program: String },
    #[error("{program} exited with status {status}")]
    NonZero { program: String, status: i32 },
}

/// Port every external program invocation goes through.
pub trait CommandRunner {
    fn run(&self, spec: &CommandSpec) -> Result<CommandOutcome, RunnerError>;
}

/// The real runner: `std::process::Command`, no shell, no `PATH`
/// inheritance, `env_clear()` before applying only the spec's own pairs.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> Result<CommandOutcome, RunnerError> {
        let program = spec.program.display().to_string();
        if !spec.program.is_absolute() {
            return Err(RunnerError::NonAbsoluteProgram { program });
        }
        let output = std::process::Command::new(&spec.program)
            .args(&spec.args)
            .env_clear()
            .envs(spec.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::null())
            .output()
            .map_err(|e| RunnerError::Spawn { program: program.clone(), reason: e.to_string() })?;

        // `status.code()` is `None` when the child was killed by a signal —
        // this is always a failure, regardless of `Expect`.
        let status = output.status.code().ok_or_else(|| RunnerError::Signaled { program: program.clone() })?;

        if spec.expect == Expect::Zero && status != 0 {
            return Err(RunnerError::NonZero { program, status });
        }

        Ok(CommandOutcome { status: Some(status), stdout: output.stdout, stderr: output.stderr })
    }
}

/// Test-only `CommandRunner` that asserts each invocation against a
/// pre-scripted queue of `(CommandSpec, Result<CommandOutcome,
/// RunnerError>)` pairs, in order. This is how every `visudo`/`sudo`/
/// `systemctl`/`systemd-run` argv is pinned and how failure injection is
/// driven, without touching real binaries (design.md §3).
#[cfg(test)]
pub(crate) struct ScriptedRunner {
    script: RefCell<VecDeque<(CommandSpec, Result<CommandOutcome, RunnerError>)>>,
}

#[cfg(test)]
impl ScriptedRunner {
    pub(crate) fn new(script: Vec<(CommandSpec, Result<CommandOutcome, RunnerError>)>) -> Self {
        ScriptedRunner { script: RefCell::new(script.into()) }
    }
}

#[cfg(test)]
impl CommandRunner for ScriptedRunner {
    fn run(&self, spec: &CommandSpec) -> Result<CommandOutcome, RunnerError> {
        let (expected, result) = self
            .script
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("ScriptedRunner: script exhausted, but got another call: {spec:?}"));
        assert_eq!(expected, *spec, "CommandSpec mismatch");
        result
    }
}

#[cfg(test)]
impl Drop for ScriptedRunner {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let remaining = self.script.borrow().len();
        assert_eq!(remaining, 0, "ScriptedRunner: script not exhausted, {remaining} command(s) remaining");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang_c_env() -> Vec<(String, String)> {
        vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]
    }

    #[test]
    fn system_runner_does_not_shell_interpret_arguments() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/echo"),
            args: vec![
                "$(whoami)".to_string(),
                ";".to_string(),
                "id".to_string(),
                "`id`".to_string(),
                "|".to_string(),
                "&&".to_string(),
                "line1\nline2".to_string(),
                "*".to_string(),
            ],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        let outcome = SystemRunner.run(&spec).unwrap();
        let stdout = String::from_utf8_lossy(&outcome.stdout);
        assert_eq!(stdout.trim_end(), "$(whoami) ; id `id` | && line1\nline2 *");
    }

    /// Writes an executable script at a RELATIVE path (no leading `/`)
    /// under the crate's test working directory (Cargo runs test
    /// binaries with `cwd` = the package root). Every test using this
    /// helper asserts `NonAbsoluteProgram` is returned *before* any
    /// spawn, so the file it writes is never exec'd and cannot hit the
    /// `ETXTBSY` race described below. Returns the relative name (as a
    /// `CommandSpec::program` value would use it),
    /// the absolute path used only for cleanup, and an absolute marker
    /// file the script touches if it ever actually runs.
    fn write_relative_marker_script(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let relative_name = PathBuf::from(format!("nopass_test_relprog_{tag}_{}_{nanos}_{n}.sh", std::process::id()));
        let absolute_path = std::env::current_dir().unwrap().join(&relative_name);
        let marker = std::env::temp_dir()
            .join(format!("nopass_test_relprog_marker_{tag}_{}_{nanos}_{n}", std::process::id()));
        std::fs::write(&absolute_path, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        let mut perms = std::fs::metadata(&absolute_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&absolute_path, perms).unwrap();
        (relative_name, absolute_path, marker)
    }

    #[test]
    fn system_runner_refuses_bare_program_name_without_spawning() {
        let (relative_name, absolute_path, marker) = write_relative_marker_script("bare");
        let spec = CommandSpec {
            program: relative_name.clone(),
            args: vec![],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        let result = SystemRunner.run(&spec);
        let _ = std::fs::remove_file(&absolute_path);
        let ran = marker.exists();
        let _ = std::fs::remove_file(&marker);

        match result {
            Err(RunnerError::NonAbsoluteProgram { program }) => {
                assert_eq!(program, relative_name.display().to_string());
            }
            other => panic!("expected RunnerError::NonAbsoluteProgram, got {other:?}"),
        }
        assert!(!ran, "marker file exists — bare program name was executed despite not being absolute");
    }

    #[test]
    fn system_runner_refuses_relative_dot_slash_program_path_without_spawning() {
        let (relative_name, absolute_path, marker) = write_relative_marker_script("dotslash");
        let dot_slash = PathBuf::from(".").join(&relative_name);
        let spec =
            CommandSpec { program: dot_slash.clone(), args: vec![], env: lang_c_env(), expect: Expect::Zero };
        let result = SystemRunner.run(&spec);
        let _ = std::fs::remove_file(&absolute_path);
        let ran = marker.exists();
        let _ = std::fs::remove_file(&marker);

        match result {
            Err(RunnerError::NonAbsoluteProgram { program }) => {
                assert_eq!(program, dot_slash.display().to_string());
            }
            other => panic!("expected RunnerError::NonAbsoluteProgram, got {other:?}"),
        }
        assert!(!ran, "marker file exists — ./relative program path was executed despite not being absolute");
    }

    #[test]
    fn system_runner_runs_absolute_program_path_normally() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/echo"),
            args: vec!["ok".to_string()],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        let outcome = SystemRunner.run(&spec).unwrap();
        assert_eq!(String::from_utf8_lossy(&outcome.stdout).trim_end(), "ok");
    }

    #[test]
    fn system_runner_clears_ambient_env_and_keeps_pkexec_uid_out_of_the_child() {
        // Adversarial names covered: PKEXEC_UID plus PATH, HOME,
        // LD_PRELOAD, LD_LIBRARY_PATH, IFS, BASH_ENV, and the SUDO_*
        // family (privilege-escalation- and shell-hijack-relevant
        // names).
        const DANGEROUS_NAMES: &[&str] = &[
            "PKEXEC_UID",
            "PATH",
            "HOME",
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "IFS",
            "BASH_ENV",
            "SUDO_USER",
            "SUDO_UID",
            "SUDO_GID",
            "SUDO_COMMAND",
        ];
        const ADVERSARIAL_FLAG: &str = "NOPASS_TEST_ENV_ADVERSARIAL_CHILD";

        if std::env::var(ADVERSARIAL_FLAG).is_ok() {
            // Inner invocation, running as the re-exec'd child below: the
            // names in `DANGEROUS_NAMES` are genuinely set in THIS
            // process's own ambient environment (via the outer test's
            // `Command::env` calls, never `std::env::set_var` — this
            // crate is `#![forbid(unsafe_code)]`). This makes the
            // assertion adversarial: it proves `env_clear()` actually
            // strips names that were present, not merely names that
            // happened to be absent already.
            // H1 (verify-report.md): driving `/bin/sh -c "env"` directly,
            // rather than writing a temp script and exec'ing it, needs no
            // file this process ever opens for writing — see
            // `system_runner_treats_signal_death_as_failure_even_under_expect_any`
            // for the full ETXTBSY race this avoids.
            let spec = CommandSpec {
                program: PathBuf::from("/bin/sh"),
                args: vec!["-c".to_string(), "env".to_string()],
                env: lang_c_env(),
                expect: Expect::Zero,
            };
            let outcome = SystemRunner.run(&spec);
            let outcome = outcome.expect("env-dump script exits 0");
            let stdout = String::from_utf8_lossy(&outcome.stdout);
            let names: Vec<&str> = stdout.lines().filter_map(|line| line.split('=').next()).collect();
            assert!(names.contains(&"LANG"), "LANG missing from child env: {stdout}");
            assert!(names.contains(&"LC_ALL"), "LC_ALL missing from child env: {stdout}");
            for dangerous in DANGEROUS_NAMES {
                assert!(
                    !names.contains(dangerous),
                    "{dangerous} leaked into child env despite env_clear(): {stdout}"
                );
            }
            return;
        }

        // Outer invocation: re-exec this same test binary, filtered down
        // to only this test, as a CHILD process that genuinely has every
        // name in `DANGEROUS_NAMES` set in its own ambient environment —
        // via `Command::env`, which configures the child's environment
        // without ever mutating this (parent) process's environment.
        let exe = std::env::current_exe().expect("current test binary path");
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("runner::tests::system_runner_clears_ambient_env_and_keeps_pkexec_uid_out_of_the_child")
            .arg("--exact")
            .arg("--nocapture")
            .env(ADVERSARIAL_FLAG, "1")
            .env("PKEXEC_UID", "1000")
            .env("PATH", "/tmp/nopass-test-adversarial-path")
            .env("HOME", "/tmp/nopass-test-adversarial-home")
            .env("LD_PRELOAD", "/tmp/nopass-test-adversarial-evil.so")
            .env("LD_LIBRARY_PATH", "/tmp/nopass-test-adversarial-evil-lib")
            .env("IFS", "$")
            .env("BASH_ENV", "/tmp/nopass-test-adversarial-evil-bashrc")
            .env("SUDO_USER", "root")
            .env("SUDO_UID", "0")
            .env("SUDO_GID", "0")
            .env("SUDO_COMMAND", "/bin/true");
        let output = cmd.output().expect("spawn adversarial child test process");
        if !output.status.success() {
            panic!(
                "adversarial child test failed (status {:?}):\nstdout: {}\nstderr: {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }

    #[test]
    fn system_runner_treats_signal_death_as_failure_even_under_expect_any() {
        // H1 (verify-report.md): this used to write its own temp
        // executable and immediately exec it. `write_temp_script` created
        // an executable file and closed it, but `Command::spawn` forks —
        // and `fork()` clones the whole process's fd table into the
        // child — so a child forked by an unrelated test thread between
        // this test's `fs::write` and its own exec could still be
        // holding that file's write descriptor open (inherited from this
        // process before this test's own close), which is enough for the
        // kernel to refuse *this* test's exec with `ETXTBSY`, even though
        // this process's own fd was already closed. Reproduced
        // independently at roughly 1 failure in 15 runs. Driving
        // `/bin/sh -c` directly needs no file this process ever opens
        // for writing, so there is nothing left to race over.
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), "kill -9 $$".to_string()],
            env: lang_c_env(),
            expect: Expect::Any,
        };
        let outcome = SystemRunner.run(&spec);
        assert!(matches!(outcome, Err(RunnerError::Signaled { .. })), "expected Signaled, got {outcome:?}");
    }

    #[test]
    fn system_runner_expect_zero_fails_on_non_zero_exit() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), "exit 7".to_string()],
            env: lang_c_env(),
            expect: Expect::Zero,
        };
        let err = SystemRunner.run(&spec).unwrap_err();
        assert_eq!(err, RunnerError::NonZero { program: "/bin/sh".to_string(), status: 7 });
    }

    #[test]
    fn system_runner_expect_any_tolerates_non_zero_exit() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), "exit 5".to_string()],
            env: lang_c_env(),
            expect: Expect::Any,
        };
        let outcome = SystemRunner.run(&spec).unwrap();
        assert_eq!(outcome.status, Some(5));
    }

    fn sample_spec(tag: &str) -> CommandSpec {
        CommandSpec {
            program: PathBuf::from(format!("/usr/bin/{tag}")),
            args: vec!["-x".to_string()],
            env: lang_c_env(),
            expect: Expect::Zero,
        }
    }

    #[test]
    fn scripted_runner_pops_front_in_order_and_returns_the_scripted_result() {
        let outcome_a = CommandOutcome { status: Some(0), stdout: b"a".to_vec(), stderr: vec![] };
        let outcome_b = CommandOutcome { status: Some(0), stdout: b"b".to_vec(), stderr: vec![] };
        let runner = ScriptedRunner::new(vec![
            (sample_spec("visudo"), Ok(outcome_a.clone())),
            (sample_spec("sudo"), Ok(outcome_b.clone())),
        ]);
        assert_eq!(runner.run(&sample_spec("visudo")).unwrap(), outcome_a);
        assert_eq!(runner.run(&sample_spec("sudo")).unwrap(), outcome_b);
    }

    #[test]
    #[should_panic(expected = "CommandSpec mismatch")]
    fn scripted_runner_panics_on_spec_mismatch() {
        let outcome = CommandOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let runner = ScriptedRunner::new(vec![(sample_spec("visudo"), Ok(outcome))]);
        let _ = runner.run(&sample_spec("systemctl"));
    }

    #[test]
    #[should_panic(expected = "script not exhausted")]
    fn scripted_runner_panics_on_drop_when_script_not_exhausted() {
        let outcome = CommandOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let _runner = ScriptedRunner::new(vec![(sample_spec("visudo"), Ok(outcome))]);
        // Deliberately never call `.run()` — the script is never consumed.
    }
}
