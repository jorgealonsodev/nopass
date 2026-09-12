//! Static assertions over the `data/icons/` install artifacts (design.md
//! §7.1; spec `tray-presence` "Three Visual States With Distinct Icon
//! Names"; tasks.md Phase 6, task 6.2).
//!
//! These six SVGs are never installed by this workspace — packaging (M4)
//! owns copying them into an icon theme directory — so nothing else in
//! the build reads their content. Deliberately plain `str` assertions:
//! no XML parser, same discipline as `nopass-helper`'s
//! `tests/data_artifacts.rs`.

use std::path::{Path, PathBuf};

const ICON_NAMES: [&str; 6] = [
    "nopass-locked",
    "nopass-locked-symbolic",
    "nopass-unlocked",
    "nopass-unlocked-symbolic",
    "nopass-unlocked-timed",
    "nopass-unlocked-timed-symbolic",
];

fn icon_file(name: &str) -> String {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/icons").join(format!("{name}.svg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read data/icons/{name}.svg at {}: {e}", path.display()))
}

#[test]
fn all_six_icon_assets_are_present() {
    for name in ICON_NAMES {
        let contents = icon_file(name);
        assert!(!contents.is_empty(), "{name}.svg must not be empty");
    }
}

#[test]
fn every_icon_has_an_svg_root_element() {
    for name in ICON_NAMES {
        let contents = icon_file(name);
        assert!(contents.trim_start().starts_with("<svg"), "{name}.svg must start with an <svg root element");
    }
}

#[test]
fn every_icon_declares_a_view_box() {
    for name in ICON_NAMES {
        let contents = icon_file(name);
        assert!(contents.contains("viewBox="), "{name}.svg must declare a viewBox attribute");
    }
}

/// Threat matrix hygiene: an SVG asset is XML, and XML that can carry a
/// `<script>` element or reach out over `http(s)` is not a safe icon
/// theme install artifact.
#[test]
fn no_icon_contains_a_script_element() {
    for name in ICON_NAMES {
        let contents = icon_file(name);
        assert!(!contents.contains("<script"), "{name}.svg must not contain a <script element");
    }
}

/// The mandatory `xmlns="http://www.w3.org/2000/svg"` namespace
/// declaration is a fixed, never-dereferenced constant, not an external
/// reference — it is stripped before the check below runs, so this test
/// still catches a real `href`/`src`/`xlink:href` pointed at the network.
#[test]
fn no_icon_references_an_http_scheme_external_resource() {
    const SVG_NAMESPACE: &str = r#"xmlns="http://www.w3.org/2000/svg""#;
    for name in ICON_NAMES {
        let contents = icon_file(name).replace(SVG_NAMESPACE, "");
        assert!(!contents.contains("http://"), "{name}.svg must not reference an http:// URL");
        assert!(!contents.contains("https://"), "{name}.svg must not reference an https:// URL");
    }
}

/// No embedded pixmaps (design.md §7.1: "Embedding pixmaps is rejected") —
/// a base64 `data:image` URI would be the SVG-native way to smuggle one in.
#[test]
fn no_icon_embeds_a_pixmap_via_a_data_uri() {
    for name in ICON_NAMES {
        let contents = icon_file(name);
        assert!(!contents.contains("data:image"), "{name}.svg must not embed a pixmap via a data:image URI");
    }
}
