//! Structural assertions for the RPM package metadata and lifecycle scriptlets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Value};

fn crate_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn manifest() -> DocumentMut {
    let path = crate_root().join("Cargo.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .parse()
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn rpm_metadata(document: &DocumentMut) -> &toml_edit::Item {
    &document["package"]["metadata"]["generate-rpm"]
}

fn asset_string(asset: &Value, key: &str) -> String {
    asset
        .as_inline_table()
        .and_then(|table| table.get(key))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("RPM asset must have a string `{key}` field"))
        .to_owned()
}

fn rpm_scriptlet(document: &DocumentMut, key: &str) -> String {
    let rpm = rpm_metadata(document);
    let script_path = rpm[key]
        .as_str()
        .unwrap_or_else(|| panic!("RPM metadata must name `{key}`"));
    let path = crate_root().join(script_path);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read RPM scriptlet {}: {error}", path.display()))
}

#[test]
fn rpm_assets_match_the_debian_layout_and_shared_helper_path() {
    let document = manifest();
    let assets = rpm_metadata(&document)["assets"]
        .as_array()
        .expect("RPM metadata must define an assets array");

    let actual: BTreeMap<_, _> = assets
        .iter()
        .map(|asset| {
            let source = asset_string(asset, "source");
            let destination = asset_string(asset, "dest");
            let mode = asset_string(asset, "mode");
            (source, (destination, mode))
        })
        .collect();

    let expected = BTreeMap::from([
        (
            "../../target/release/nopass".to_owned(),
            ("/usr/bin/nopass".to_owned(), "755".to_owned()),
        ),
        (
            "../../target/release/nopass-helper".to_owned(),
            ("/usr/libexec/nopass-helper".to_owned(), "755".to_owned()),
        ),
        (
            "../../data/com.enfoquestic.nopass.policy".to_owned(),
            (
                "/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy".to_owned(),
                "644".to_owned(),
            ),
        ),
        (
            "../../data/nopass.tmpfiles.conf".to_owned(),
            ("/usr/lib/tmpfiles.d/nopass.conf".to_owned(), "644".to_owned()),
        ),
        (
            "../../data/nopass-cleanup.service".to_owned(),
            (
                "/usr/lib/systemd/system/nopass-cleanup.service".to_owned(),
                "644".to_owned(),
            ),
        ),
        (
            "../../data/icons/*.svg".to_owned(),
            (
                "/usr/share/icons/hicolor/scalable/apps/".to_owned(),
                "644".to_owned(),
            ),
        ),
        (
            "../../data/nopass.desktop".to_owned(),
            ("/usr/share/applications/nopass.desktop".to_owned(), "644".to_owned()),
        ),
    ]);

    assert_eq!(actual, expected, "RPM assets must preserve the Debian layout");
}

#[test]
fn rpm_scriptlets_follow_the_debian_lifecycle_without_enabling_autostart() {
    let document = manifest();
    let post_install = rpm_scriptlet(&document, "post_install_script");
    let pre_uninstall = rpm_scriptlet(&document, "pre_uninstall_script");
    let post_uninstall = rpm_scriptlet(&document, "post_uninstall_script");

    assert!(
        post_install.contains("systemd-tmpfiles --create /usr/lib/tmpfiles.d/nopass.conf"),
        "post-install must create the runtime directory through tmpfiles"
    );
    assert!(
        !post_install.contains("systemctl enable nopass-cleanup.service"),
        "package install must not enable the user-owned cleanup service"
    );

    assert!(
        pre_uninstall.contains("/usr/libexec/nopass-helper expire --boot"),
        "pre-uninstall must use the installed helper to revoke grants"
    );
    assert!(
        pre_uninstall.contains("\"$1\" = \"0\""),
        "pre-uninstall cleanup must run only for final erase, not upgrade"
    );

    assert!(
        post_uninstall.contains("rm -rf /run/nopass"),
        "post-uninstall must remove the non-packaged runtime directory"
    );
    assert!(
        post_uninstall.contains("\"$1\" = \"0\""),
        "runtime directory removal must run only for final erase"
    );

    let rpm_scripts = crate_root().join("rpm");
    for entry in std::fs::read_dir(&rpm_scripts).expect("RPM scriptlet directory must exist") {
        let path = entry.expect("RPM scriptlet directory entry must be readable").path();
        if path.is_file() {
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
            assert!(
                !contents.contains("systemctl enable nopass-cleanup.service"),
                "{} must not enable the user-owned cleanup service",
                path.display()
            );
        }
    }
}

#[test]
fn rpm_lifecycle_lane_wires_runner_and_fixture_to_the_same_selftest_toggle() {
    const SELFTEST_ENV: &str = "NOPASS_LANE_SELFTEST_ROGUE_RULE";
    let root = crate_root().join("../..");
    let runner_path = root.join("scripts/run-lane-rpm.sh");
    let fixture_path = root.join("tests/containers/fixtures/rpm-lifecycle.sh");
    let runner = std::fs::read_to_string(&runner_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", runner_path.display()));
    let fixture = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", fixture_path.display()));

    assert!(runner.contains("command -v docker"), "runner must detect Docker");
    assert!(runner.contains("command -v podman"), "runner must detect Podman");
    assert!(runner.contains("Containerfile.rpm"), "runner must build the RPM image");
    assert!(
        runner.contains("tests/containers/fixtures/rpm-lifecycle.sh"),
        "runner must execute the RPM lifecycle fixture"
    );
    assert!(
        runner.contains(&format!("-e {SELFTEST_ENV}")),
        "runner must pass the self-test toggle into the container"
    );
    assert!(
        fixture.contains(&format!("${{{SELFTEST_ENV}:-0}}")),
        "fixture must use the same self-test toggle"
    );
    assert!(
        fixture.contains("/etc/sudoers.d/90-nopass-"),
        "fixture must inspect live NoPass sudoers rules"
    );
}
