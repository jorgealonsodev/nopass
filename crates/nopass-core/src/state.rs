//! `HelperStatus`: the wire type shared byte-for-byte between the `status`
//! subcommand's stdout and `/run/nopass/<uid>.state`.
//!
//! `expires` is internally tagged (`#[serde(tag = "kind")]`) via
//! [`crate::expiry::Expiry`], so this contract renders as
//! `{"kind":"never"}` / `{"kind":"reboot"}` / `{"kind":"at","epoch":N}` /
//! `null` — never as a bare tuple encoding.

use serde::{Deserialize, Serialize};

use crate::expiry::Expiry;
use crate::header::RuleHeader;

/// Wire schema version. A reader (the M2 tray) MUST reject `schema != 1`.
pub const SCHEMA_VERSION: u32 = 1;

/// The status of one uid's NoPass grant, serialized identically for
/// `status` stdout and the state file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelperStatus {
    pub schema: u32,
    pub uid: u32,
    pub user: String,
    pub active: bool,
    pub expires: Option<Expiry>,
    pub rule_path: String,
    pub updated_at: u64,
}

impl HelperStatus {
    /// Builds an active-grant status from a parsed rule-file header.
    pub fn active_from(uid: u32, header: &RuleHeader, rule_path: String, now: u64) -> Self {
        HelperStatus {
            schema: SCHEMA_VERSION,
            uid,
            user: header.user.clone(),
            active: true,
            expires: Some(header.expires),
            rule_path,
            updated_at: now,
        }
    }

    /// Builds an inactive status — no rule present, no expiry.
    pub fn inactive(uid: u32, user: String, rule_path: String, now: u64) -> Self {
        HelperStatus {
            schema: SCHEMA_VERSION,
            uid,
            user,
            active: false,
            expires: None,
            rule_path,
            updated_at: now,
        }
    }

    /// Serializes as one compact JSON object followed by `\n`, ready to
    /// write to stdout or to a state file.
    pub fn to_json_line(&self) -> String {
        format!("{}\n", serde_json::to_string(self).expect("HelperStatus always serializes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(user: &str, expires: Expiry) -> RuleHeader {
        RuleHeader { user: user.to_string(), expires }
    }

    #[test]
    fn active_from_serializes_the_exact_golden_json_for_at_expiry() {
        let h = header("jorge", Expiry::At { epoch: 1_789_000_000 });
        let status = HelperStatus::active_from(
            1000,
            &h,
            "/etc/sudoers.d/90-nopass-1000".to_string(),
            1_789_000_000,
        );
        assert_eq!(
            status.to_json_line(),
            "{\"schema\":1,\"uid\":1000,\"user\":\"jorge\",\"active\":true,\"expires\":{\"kind\":\"at\",\"epoch\":1789000000},\"rule_path\":\"/etc/sudoers.d/90-nopass-1000\",\"updated_at\":1789000000}\n"
        );
    }

    #[test]
    fn inactive_serializes_false_active_and_null_expires() {
        let status = HelperStatus::inactive(
            1000,
            "jorge".to_string(),
            "/etc/sudoers.d/90-nopass-1000".to_string(),
            1_789_000_500,
        );
        assert!(!status.active);
        assert_eq!(status.expires, None);
        assert_eq!(
            status.to_json_line(),
            "{\"schema\":1,\"uid\":1000,\"user\":\"jorge\",\"active\":false,\"expires\":null,\"rule_path\":\"/etc/sudoers.d/90-nopass-1000\",\"updated_at\":1789000500}\n"
        );
    }

    #[test]
    fn active_and_inactive_status_share_the_identical_serializer_shape() {
        let h = header("ana", Expiry::Never);
        let active = HelperStatus::active_from(2000, &h, "/etc/sudoers.d/90-nopass-2000".to_string(), 42);
        let inactive =
            HelperStatus::inactive(2000, "ana".to_string(), "/etc/sudoers.d/90-nopass-2000".to_string(), 42);
        let active_json: serde_json::Value = serde_json::from_str(active.to_json_line().trim_end()).unwrap();
        let inactive_json: serde_json::Value = serde_json::from_str(inactive.to_json_line().trim_end()).unwrap();
        assert_eq!(
            active_json.as_object().unwrap().keys().collect::<Vec<_>>(),
            inactive_json.as_object().unwrap().keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn to_json_line_round_trips_through_serde_json() {
        let h = header("ana.perez", Expiry::Reboot);
        let status = HelperStatus::active_from(2000, &h, "/etc/sudoers.d/90-nopass-2000".to_string(), 99);
        let line = status.to_json_line();
        let parsed: HelperStatus = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(parsed, status);
    }
}
