//! Structural assertions for the AUR PKGBUILD and its installed layout.

use std::path::{Path, PathBuf};

use nopass_core::paths::HELPER_PATH;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn aur_pkgbuild_preserves_shared_helper_path_and_user_owned_activation() {
    let root = repository_root();
    let path = root.join("PKGBUILD");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));

    assert!(contents.contains("pkgver=0.1.0"), "AUR version must match the release");
    assert!(
        contents.contains("https://github.com/jorgealonsodev/nopass")
            && contents.contains("#tag=v${pkgver}"),
        "AUR source must use the matching GitHub release tag"
    );
    assert!(
        contents.contains("cargo build --release --locked --workspace"),
        "PKGBUILD must build the workspace with Cargo"
    );

    let helper_install = contents
        .lines()
        .find(|line| {
            line.trim_start().starts_with("install ")
                && line.contains("target/release/nopass-helper")
        })
        .expect("PKGBUILD must install the helper binary");
    let destination = helper_install
        .split_whitespace()
        .last()
        .expect("helper install command must have a destination")
        .trim_matches('"')
        .strip_prefix("$pkgdir")
        .expect("helper install destination must be rooted in pkgdir");
    assert_eq!(
        destination, HELPER_PATH,
        "AUR helper destination must match the compiled/pkexec path"
    );

    for required in [
        "install -Dm755 target/release/nopass \"$pkgdir/usr/bin/nopass\"",
        "install -Dm644 data/com.enfoquestic.nopass.policy \"$pkgdir/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy\"",
        "install -Dm644 data/nopass.tmpfiles.conf \"$pkgdir/usr/lib/tmpfiles.d/nopass.conf\"",
        "install -Dm644 data/nopass-cleanup.service \"$pkgdir/usr/lib/systemd/system/nopass-cleanup.service\"",
        "install -m644 data/icons/*.svg \"$pkgdir/usr/share/icons/hicolor/scalable/apps/\"",
        "install -Dm644 data/nopass.desktop \"$pkgdir/usr/share/applications/nopass.desktop\"",
        "install -Dm644 LICENSE \"$pkgdir/usr/share/licenses/$pkgname/LICENSE\"",
    ] {
        assert!(contents.contains(required), "PKGBUILD is missing: {required}");
    }

    let comments = contents
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix('#'))
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    assert!(
        comments.contains("arch")
            && comments.contains("pkexec")
            && comments.contains("resolved helper path"),
        "PKGBUILD must explain why Arch keeps the resolved pkexec helper path"
    );
    assert!(
        !contents.contains("/usr/lib/nopass"),
        "PKGBUILD must not rewrite the helper path to /usr/lib/nopass"
    );
    assert!(
        !contents.contains("ln -s"),
        "PKGBUILD must not add a helper-path symlink"
    );
    assert!(
        !contents.contains("systemctl enable nopass-cleanup.service"),
        "package installation must not enable the user-owned cleanup service"
    );
    assert!(
        !contents.lines().any(|line| {
            !line.trim_start().starts_with('#') && line.contains("autostart/")
        }),
        "PKGBUILD must not install an autostart entry"
    );
}
