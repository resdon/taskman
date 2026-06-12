pkgname=taskman
pkgver=0.1.0
pkgrel=1
pkgdesc="A custom Rust-based Task Manager"
arch=('x86_64')
url="https://github.com/resdon/taskman"
license=('MIT')
depends=('gcc-libs' 'nvidia-utils')
makedepends=('rust' 'cargo')
source=("git+https://github.com/resdon/taskman.git")
sha256sums=('SKIP')

build() {
    cd "$pkgname"
    cargo build --release --locked
}

package() {
    cd "$pkgname"
    # Install the binary
    install -Dm755 "target/release/taskman" "$pkgdir/usr/bin/taskman"
    
    # Install the icon (ensure this path matches where it is in your repo)
    install -Dm644 "assets/icon.svg" "$pkgdir/usr/share/icons/hicolor/scalable/apps/taskman.svg"
    
    # Create and install the desktop file
    install -Dm644 /dev/stdin "$pkgdir/usr/share/applications/taskman.desktop" <<EOF
[Desktop Entry]
Name=Taskman
Exec=/usr/bin/taskman
Icon=taskman
Type=Application
Categories=System;Monitor;
Terminal=false
StartupWMClass=taskman
EOF
}
