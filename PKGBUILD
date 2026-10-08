pkgname=nopass
pkgver=0.2.0
pkgrel=1
pkgdesc="Grants and revokes passwordless sudo for the current session's user through polkit"
arch=('x86_64')
url="https://github.com/jorgealonsodev/nopass"
license=('MIT')
depends=('polkit' 'sudo' 'systemd')
makedepends=('cargo' 'git')
source=("$pkgname::git+https://github.com/jorgealonsodev/nopass.git#tag=v${pkgver}")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname"
    cargo build --release --locked --workspace
}

package() {
    cd "$srcdir/$pkgname"

    install -Dm755 target/release/nopass "$pkgdir/usr/bin/nopass"

    # Arch often places helpers under /usr/lib, but pkexec requires the resolved
    # helper path to match the polkit policy and compiled HELPER_PATH. Keep this
    # path unchanged; a rewrite or symlink would break authorization.
    install -Dm755 target/release/nopass-helper "$pkgdir/usr/libexec/nopass-helper"

    install -Dm644 data/com.enfoquestic.nopass.policy "$pkgdir/usr/share/polkit-1/actions/com.enfoquestic.nopass.policy"
    install -Dm644 data/nopass.tmpfiles.conf "$pkgdir/usr/lib/tmpfiles.d/nopass.conf"
    install -Dm644 data/nopass-cleanup.service "$pkgdir/usr/lib/systemd/system/nopass-cleanup.service"

    install -d -m755 "$pkgdir/usr/share/icons/hicolor/scalable/apps"
    install -m644 data/icons/*.svg "$pkgdir/usr/share/icons/hicolor/scalable/apps/"

    install -Dm644 data/nopass.desktop "$pkgdir/usr/share/applications/nopass.desktop"
    install -Dm644 README.md "$pkgdir/usr/share/doc/nopass/README.md"
    install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"

    # Cleanup scheduling and autostart are user-owned; do not enable the
    # cleanup service or install autostart entries from the package.
}
