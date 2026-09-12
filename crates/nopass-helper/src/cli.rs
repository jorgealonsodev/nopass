//! CLI surface for `nopass-helper`: the fixed four-subcommand, closed-flag
//! parser (helper-cli spec, "Fixed Subcommand and Flag Surface").
//!
//! No `allow_external_subcommands`, `allow_hyphen_values`, or
//! `trailing_var_arg` — anything not explicitly declared here is rejected
//! by clap at parse time (exit 2) before any privileged handler runs.

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
}
