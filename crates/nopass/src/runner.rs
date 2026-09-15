//! `CommandRunner` port: every external process spawn goes through this
//! trait so exact argv can be pinned in tests without touching the real
//! binaries (design.md §4.1; threat matrix "External command composition").
//!
//! Same shape as `nopass-helper::runner`, minus `Expect` — the tray
//! inspects every exit status itself rather than asking the port to
//! enforce a zero-exit contract — and plus `Send + Sync + 'static`, so a
//! runner can be shared across the dedicated thread `run_off_reactor`
//! spawns per privileged invocation (design.md §4.2).

#[cfg(test)]
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
#[cfg(test)]
use std::sync::Mutex;

/// One external command invocation: absolute program path, argv, and the
/// minimal environment it is allowed to see. Never a shell string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Applied after `env_clear()` — this is the *entire* child
    /// environment, nothing is inherited.
    pub env: Vec<(String, String)>,
}

/// The result of running a `CommandSpec`. The tray reads `status` itself
/// (0/1/2/10-17/126/127/…) rather than delegating a zero-exit contract to
/// this port, so — unlike `nopass-helper::runner::CommandOutcome` — this
/// type carries no `Expect`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnOutcome {
    pub status: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunnerError {
    /// The spec's `program` was not an absolute path. `env_clear()` alone
    /// does not guarantee no `PATH`-style lookup: glibc's `execvp` falls
    /// back to `confstr(_CS_PATH)` when `PATH` is entirely absent from the
    /// child env, so a bare or relative program name can still resolve and
    /// run. Returned BEFORE anything is spawned.
    #[error("program path is not absolute: {program}")]
    NonAbsoluteProgram { program: String },
    #[error("failed to spawn {program}: {reason}")]
    Spawn { program: String, reason: String },
    /// `status: None` — the child died by signal. Always a failure, never
    /// treated as success, regardless of what the caller expected.
    #[error("{program} was killed by a signal")]
    Signaled { program: String },
}

/// Port every external program invocation goes through.
pub trait CommandRunner: Send + Sync + 'static {
    fn run(&self, spec: &CommandSpec) -> Result<SpawnOutcome, RunnerError>;
}

/// The real runner: `std::process::Command`, no shell, no `PATH`
/// inheritance, `env_clear()` before applying only the spec's own pairs.
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, spec: &CommandSpec) -> Result<SpawnOutcome, RunnerError> {
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

        match output.status.code() {
            Some(status) => Ok(SpawnOutcome { status: Some(status), stdout: output.stdout, stderr: output.stderr }),
            // `status.code()` is `None` when the child was killed by a
            // signal — always a failure, never treated as success.
            None => Err(RunnerError::Signaled { program }),
        }
    }
}

/// Runs `spec` on a dedicated OS thread, joined implicitly when it ends,
/// and returns the result over an async channel the reactor is already
/// polling. No blocking call ever executes on the reactor thread — a
/// `pkexec` waiting on a human for up to 60 s must never occupy a shared
/// pool slot the 60 s probe also needs (design.md §4.2).
pub async fn run_off_reactor(
    runner: Arc<dyn CommandRunner>,
    spec: CommandSpec,
) -> Result<SpawnOutcome, RunnerError> {
    let program = spec.program.display().to_string();
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(runner.run(&spec));
    });
    rx.recv().await.unwrap_or(Err(RunnerError::Spawn {
        program,
        reason: "the off-reactor thread ended without sending a result".to_string(),
    }))
}

/// Test-only `CommandRunner` that asserts each invocation against a
/// pre-scripted queue of `(CommandSpec, Result<SpawnOutcome, RunnerError>)`
/// pairs, in order. Backed by a `Mutex`, not a `RefCell` as
/// `nopass-helper`'s sibling uses: `CommandRunner: Send + Sync` means a
/// type usable from `run_off_reactor`'s dedicated thread cannot rely on
/// interior mutability without synchronization.
#[cfg(test)]
pub(crate) struct ScriptedRunner {
    script: Mutex<VecDeque<(CommandSpec, Result<SpawnOutcome, RunnerError>)>>,
}

#[cfg(test)]
impl ScriptedRunner {
    pub(crate) fn new(script: Vec<(CommandSpec, Result<SpawnOutcome, RunnerError>)>) -> Self {
        ScriptedRunner { script: Mutex::new(script.into()) }
    }
}

#[cfg(test)]
impl CommandRunner for ScriptedRunner {
    fn run(&self, spec: &CommandSpec) -> Result<SpawnOutcome, RunnerError> {
        let (expected, result) = self
            .script
            .lock()
            .unwrap()
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
        let remaining = self.script.lock().unwrap().len();
        assert_eq!(remaining, 0, "ScriptedRunner: script not exhausted, {remaining} command(s) remaining");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Writes an executable script at a RELATIVE path (no leading `/`)
    /// under the crate's test working directory (Cargo runs test binaries
    /// with `cwd` = the package root). Returns the relative name (as a
    /// `CommandSpec::program` value would use it), the absolute path used
    /// only for cleanup, and an absolute marker file the script touches
    /// if it ever actually runs.
    fn write_relative_marker_script(tag: &str) -> (PathBuf, PathBuf, PathBuf) {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let relative_name =
            PathBuf::from(format!("nopass_tray_test_relprog_{tag}_{}_{nanos}_{n}.sh", std::process::id()));
        let absolute_path = std::env::current_dir().unwrap().join(&relative_name);
        let marker = std::env::temp_dir()
            .join(format!("nopass_tray_test_relprog_marker_{tag}_{}_{nanos}_{n}", std::process::id()));
        std::fs::write(&absolute_path, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        let mut perms = std::fs::metadata(&absolute_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&absolute_path, perms).unwrap();
        (relative_name, absolute_path, marker)
    }

    fn env_c() -> Vec<(String, String)> {
        vec![("LANG".to_string(), "C".to_string()), ("LC_ALL".to_string(), "C".to_string())]
    }

    #[test]
    fn system_runner_refuses_a_relative_program_without_spawning() {
        let (relative_name, absolute_path, marker) = write_relative_marker_script("bare");
        let spec = CommandSpec { program: relative_name.clone(), args: vec![], env: env_c() };
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
        assert!(!ran, "marker file exists — a non-absolute program was executed despite the guard");
    }

    #[test]
    fn system_runner_treats_signal_death_as_a_failure() {
        // H1 (verify-report.md): this used to write its own temp
        // executable and immediately exec it, which raced every other
        // test thread's own `Command::spawn` — `fork()` clones the whole
        // process's fd table into the child, so a child forked by an
        // unrelated thread between this test's `fs::write` and its own
        // exec could still hold this file's write descriptor open long
        // enough to make our exec fail with `ETXTBSY`, even though this
        // test's own writer fd was already closed. Driving `/bin/sh -c`
        // directly needs no file this process ever opens for writing, so
        // the race has no file to race over.
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), "kill -9 $$".to_string()],
            env: env_c(),
        };
        let result = SystemRunner.run(&spec);
        match result {
            Err(RunnerError::Signaled { .. }) => {}
            other => panic!("expected RunnerError::Signaled, got {other:?}"),
        }
    }

    #[test]
    fn system_runner_runs_an_absolute_program_and_reports_its_real_exit_status() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/sh"),
            args: vec!["-c".to_string(), "exit 7".to_string()],
            env: env_c(),
        };
        let outcome = SystemRunner.run(&spec).expect("a signal-free exit is Ok, whatever the status code");
        assert_eq!(outcome.status, Some(7), "SystemRunner must report the real status, never enforce zero-exit");
    }

    #[test]
    fn system_runner_never_shell_interprets_arguments() {
        let spec = CommandSpec {
            program: PathBuf::from("/bin/echo"),
            args: vec!["$(whoami)".to_string(), ";".to_string(), "id".to_string(), "|".to_string()],
            env: env_c(),
        };
        let outcome = SystemRunner.run(&spec).unwrap();
        assert_eq!(String::from_utf8_lossy(&outcome.stdout).trim_end(), "$(whoami) ; id |");
    }

    #[test]
    fn scripted_runner_pops_front_in_order_and_returns_the_scripted_result() {
        let outcome_a = SpawnOutcome { status: Some(0), stdout: b"a".to_vec(), stderr: vec![] };
        let outcome_b = SpawnOutcome { status: Some(1), stdout: b"b".to_vec(), stderr: vec![] };
        let spec_a = CommandSpec {
            program: PathBuf::from("/usr/bin/sudo"),
            args: vec!["-k".into(), "-n".into(), "true".into()],
            env: env_c(),
        };
        let spec_b = CommandSpec { program: PathBuf::from("/usr/bin/pkexec"), args: vec![], env: vec![] };
        let runner = ScriptedRunner::new(vec![
            (spec_a.clone(), Ok(outcome_a.clone())),
            (spec_b.clone(), Ok(outcome_b.clone())),
        ]);
        assert_eq!(runner.run(&spec_a).unwrap(), outcome_a);
        assert_eq!(runner.run(&spec_b).unwrap(), outcome_b);
    }

    #[test]
    #[should_panic(expected = "CommandSpec mismatch")]
    fn scripted_runner_panics_on_spec_mismatch() {
        let outcome = SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let spec = CommandSpec { program: PathBuf::from("/usr/bin/sudo"), args: vec![], env: vec![] };
        let other = CommandSpec { program: PathBuf::from("/usr/bin/pkexec"), args: vec![], env: vec![] };
        let runner = ScriptedRunner::new(vec![(spec, Ok(outcome))]);
        let _ = runner.run(&other);
    }

    #[test]
    #[should_panic(expected = "script not exhausted")]
    fn scripted_runner_panics_on_drop_when_script_not_exhausted() {
        let outcome = SpawnOutcome { status: Some(0), stdout: vec![], stderr: vec![] };
        let spec = CommandSpec { program: PathBuf::from("/usr/bin/sudo"), args: vec![], env: vec![] };
        let _runner = ScriptedRunner::new(vec![(spec, Ok(outcome))]);
        // Deliberately never call `.run()` — the script is never consumed.
    }

    #[test]
    fn run_off_reactor_returns_the_runner_result_via_a_dedicated_thread() {
        let outcome = SpawnOutcome { status: Some(0), stdout: b"ok".to_vec(), stderr: vec![] };
        let spec = CommandSpec { program: PathBuf::from("/usr/bin/true"), args: vec![], env: vec![] };
        let runner: Arc<dyn CommandRunner> = Arc::new(ScriptedRunner::new(vec![(spec.clone(), Ok(outcome.clone()))]));
        let result = futures_lite::future::block_on(run_off_reactor(runner, spec));
        assert_eq!(result, Ok(outcome));
    }
}
