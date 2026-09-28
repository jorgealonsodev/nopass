//! Structural assertions for end-user documentation and packaged README assets.

use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Value};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn manifest() -> DocumentMut {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .parse()
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

#[test]
fn end_user_readme_has_the_required_action_sections() {
    let path = repository_root().join("README.md");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

    for heading in [
        "## Use the tray",
        "## Headless use",
        "### Grant access",
        "### Revoke access",
        "### Inspect access",
        "## Autostart and cleanup",
        "## Troubleshooting",
    ] {
        assert!(
            contents.lines().any(|line| line.trim() == heading),
            "README.md must include the `{heading}` section"
        );
    }
}

#[test]
fn deb_rpm_and_aur_install_the_readme_under_the_shared_doc_path() {
    const SOURCE: &str = "../../README.md";
    const DEB_DESTINATION: &str = "usr/share/doc/nopass/README.md";
    const RPM_DESTINATION: &str = "/usr/share/doc/nopass/README.md";

    let document = manifest();
    let deb_assets = document["package"]["metadata"]["deb"]["assets"]
        .as_array()
        .expect("Debian metadata must define an assets array");
    let has_deb_readme = deb_assets.iter().any(|asset| {
        let Some(fields) = asset.as_array() else {
            return false;
        };
        fields.get(0).and_then(Value::as_str) == Some(SOURCE)
            && fields.get(1).and_then(Value::as_str) == Some(DEB_DESTINATION)
            && fields.get(2).and_then(Value::as_str) == Some("644")
    });
    assert!(
        has_deb_readme,
        "Debian metadata must install README.md at /{DEB_DESTINATION}"
    );

    let rpm_assets = document["package"]["metadata"]["generate-rpm"]["assets"]
        .as_array()
        .expect("RPM metadata must define an assets array");
    let has_rpm_readme = rpm_assets.iter().any(|asset| {
        let Some(table) = asset.as_inline_table() else {
            return false;
        };
        table.get("source").and_then(Value::as_str) == Some(SOURCE)
            && table.get("dest").and_then(Value::as_str) == Some(RPM_DESTINATION)
            && table.get("mode").and_then(Value::as_str) == Some("644")
    });
    assert!(
        has_rpm_readme,
        "RPM metadata must install README.md at {RPM_DESTINATION}"
    );

    let pkgbuild_path = repository_root().join("PKGBUILD");
    let pkgbuild = std::fs::read_to_string(&pkgbuild_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", pkgbuild_path.display())
    });
    assert!(
        pkgbuild.lines().any(|line| {
            !line.trim_start().starts_with('#')
                && line.contains("install -Dm644 README.md")
                && line.contains("$pkgdir/usr/share/doc/nopass/README.md")
        }),
        "PKGBUILD must install README.md at {RPM_DESTINATION}"
    );
}
