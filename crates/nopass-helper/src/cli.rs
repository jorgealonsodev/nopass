//! CLI surface for `nopass-helper`: the fixed seven-subcommand, closed-flag
//! parser (helper-cli spec, "Fixed Subcommand and Flag Surface").
//!
//! No `allow_external_subcommands`, `allow_hyphen_values`, or
//! `trailing_var_arg` — anything not explicitly declared here is rejected
//! by clap at parse time (exit 2) before any privileged handler runs.
//!
//! `grant`, `revoke`, and `inspect` (design.md §1, m3a-headless-grant) are
//! flat siblings of the original four, not verbs of a hand-written `admin`
//! dispatch layer — an `admin --action <verb>` form would move the verb
//! out of this validated surface, so it is rejected as a design (helper-cli
//! "An admin --action dispatch form is rejected"). Each of the three
//! requires an explicit `--uid`; no environment variable substitutes for
//! it on these subcommands.

use clap::{ArgGroup, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "nopass-helper", version, disable_help_subcommand = true)]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Debug, PartialEq, Eq, Subcommand)]
pub enum Cmd {
    #[command(group(ArgGroup::new("when").multiple(false).args(["until", "until_reboot"])))]
    Enable {
        #[arg(long)]
        until: Option<u64>,
        #[arg(long = "until-reboot")]
        until_reboot: bool,
    },
    Disable,
    Status,
    #[command(group(ArgGroup::new("target").required(true).multiple(true).args(["uid", "boot"])))]
    Expire {
        #[arg(long)]
        uid: Option<u32>,
        #[arg(long)]
        boot: bool,
    },
    /// Root-context grant of a target uid, per an explicit `--uid`
    /// (helper-cli "Required Target UID on Headless Subcommands"). The
    /// `grant_when` group name is distinct from `Enable`'s `when` — clap
    /// `ArgGroup` names are global, so reusing it would collide.
    #[command(group(ArgGroup::new("grant_when").multiple(false).args(["until", "until_reboot"])))]
    Grant {
        #[arg(long, required = true)]
        uid: u32,
        #[arg(long)]
        until: Option<u64>,
        #[arg(long = "until-reboot")]
        until_reboot: bool,
    },
    /// Root-context revoke of a target uid, per an explicit `--uid`.
    Revoke {
        #[arg(long, required = true)]
        uid: u32,
    },
    /// Root-context inspection of a target uid, per an explicit `--uid`.
    Inspect {
        #[arg(long, required = true)]
        uid: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = vec!["nopass-helper"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full)
    }

    #[test]
    fn a_documented_enable_invocation_parses_successfully() {
        let cli = parse(&["enable", "--until", "100"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Enable { until: Some(100), until_reboot: false });
    }

    #[test]
    fn unknown_subcommand_is_rejected_with_exit_code_2() {
        let err = parse(&["foo"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn unknown_flag_on_a_known_subcommand_is_rejected_with_exit_code_2() {
        let err = parse(&["enable", "--bogus"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn until_and_until_reboot_together_are_rejected_with_exit_code_2() {
        let err = parse(&["enable", "--until", "100", "--until-reboot"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn until_reboot_alone_parses_with_until_reboot_true_and_until_none() {
        let cli = parse(&["enable", "--until-reboot"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Enable { until: None, until_reboot: true });
    }

    #[test]
    fn expire_with_neither_uid_nor_boot_is_rejected_with_exit_code_2() {
        let err = parse(&["expire"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn expire_with_uid_parses_successfully() {
        let cli = parse(&["expire", "--uid", "1000"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Expire { uid: Some(1000), boot: false });
    }

    #[test]
    fn expire_with_boot_parses_successfully() {
        let cli = parse(&["expire", "--boot"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Expire { uid: None, boot: true });
    }

    #[test]
    fn disable_and_status_parse_with_no_flags() {
        assert_eq!(parse(&["disable"]).unwrap().cmd, Cmd::Disable);
        assert_eq!(parse(&["status"]).unwrap().cmd, Cmd::Status);
    }

    // --- task 5.2: a documented grant invocation parses successfully
    // (helper-cli "A documented grant invocation parses successfully") ------

    #[test]
    fn a_documented_grant_invocation_parses_successfully() {
        let cli = parse(&["grant", "--uid", "1000", "--until", "100"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Grant { uid: 1000, until: Some(100), until_reboot: false });
    }

    // --- task 5.3: an `admin --action` dispatch form is rejected
    // (helper-cli "An admin --action dispatch form is rejected") ------------

    #[test]
    fn admin_action_dispatch_form_is_rejected_with_exit_code_2() {
        let err = parse(&["admin", "--action", "grant", "--uid", "1000"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    // --- task 5.4: `grant`/`revoke`/`inspect` each require `--uid`
    // (helper-cli "Required Target UID on Headless Subcommands") ------------

    #[test]
    fn grant_without_uid_is_rejected_with_exit_code_2() {
        let err = parse(&["grant"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn revoke_and_inspect_without_uid_are_each_rejected_with_exit_code_2() {
        for args in [["revoke"].as_slice(), ["inspect"].as_slice()] {
            let err = parse(args).unwrap_err();
            assert_eq!(err.exit_code(), 2);
        }
    }

    // --- task 5.5: `revoke --uid`/`inspect --uid` parse with only that
    // flag; `grant --until-reboot` parses to Reboot; both duration flags
    // together on `grant` are rejected (helper-cli "revoke --uid and
    // inspect --uid parse with only that flag", "grant --until-reboot alone
    // parses to Reboot", "Both duration flags together on grant are
    // rejected") -------------------------------------------------------------

    #[test]
    fn revoke_uid_and_inspect_uid_parse_with_only_that_flag() {
        assert_eq!(parse(&["revoke", "--uid", "1000"]).unwrap().cmd, Cmd::Revoke { uid: 1000 });
        assert_eq!(parse(&["inspect", "--uid", "1000"]).unwrap().cmd, Cmd::Inspect { uid: 1000 });
    }

    #[test]
    fn grant_until_reboot_alone_parses_to_reboot() {
        let cli = parse(&["grant", "--uid", "1000", "--until-reboot"]).unwrap();
        assert_eq!(cli.cmd, Cmd::Grant { uid: 1000, until: None, until_reboot: true });
    }

    #[test]
    fn grant_both_duration_flags_together_are_rejected_with_exit_code_2() {
        let err = parse(&["grant", "--uid", "1000", "--until", "100", "--until-reboot"]).unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    // --- orchestrator-requested negative control: an unknown flag on each
    // new subcommand is rejected the same way `enable --bogus` already is
    // (helper-cli "Unknown flag on a known subcommand is rejected") --------

    #[test]
    fn unknown_flag_on_each_new_subcommand_is_rejected_with_exit_code_2() {
        for args in [
            ["grant", "--uid", "1000", "--bogus"].as_slice(),
            ["revoke", "--uid", "1000", "--bogus"].as_slice(),
            ["inspect", "--uid", "1000", "--bogus"].as_slice(),
        ] {
            let err = parse(args).unwrap_err();
            assert_eq!(err.exit_code(), 2);
        }
    }
}
