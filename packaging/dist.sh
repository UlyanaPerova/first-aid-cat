#!/bin/bash
# Release archive from an already built binary — the same locally and in CI.
#   packaging/dist.sh linux   — engine/target/release/aptechka                         → dist/first-aid-cat-VER-linux-x86_64.tar.gz
#   packaging/dist.sh windows — engine/target/x86_64-pc-windows-gnu/release/aptechka.exe → dist/first-aid-cat-VER-windows-x86_64.zip
#   packaging/dist.sh macos   — "macos/build/First Aid Cat.app"                         → dist/first-aid-cat-VER-macos-ARCH.zip
# The version is taken from engine/Cargo.toml.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' engine/Cargo.toml | head -1)
DOCS=(LICENSE NOTICE LICENSE-ART.md THIRD_PARTY_NOTICES.md PRIVACY.md)
URL="https://github.com/UlyanaPerova/first-aid-cat"
mkdir -p dist

case "${1:-}" in
linux)
    N="first-aid-cat-$VER-linux-x86_64"; D="dist/$N"
    rm -rf "$D"; mkdir -p "$D"
    cp engine/target/release/aptechka packaging/linux/install.sh assets/aptechka.png "$D/"
    for f in "${DOCS[@]}"; do [ -f "$f" ] && cp "$f" "$D/"; done
    cat > "$D/README.txt" <<EOF
First Aid Cat $VER for Linux (x86-64, glibc 2.35+, X11 or XWayland)

  ./install.sh              install to ~/.local/bin, menu entry, widget at login
  ./install.sh --no-widget  the same without the widget at login
  ./install.sh --uninstall  remove
  ./aptechka --help         command line

Project page, documentation and source: $URL
EOF
    tar -C dist -czf "dist/$N.tar.gz" "$N"
    echo "dist/$N.tar.gz"
    ;;
windows)
    N="first-aid-cat-$VER-windows-x86_64"; D="dist/$N"
    rm -rf "$D"; mkdir -p "$D"
    cp engine/target/x86_64-pc-windows-gnu/release/aptechka.exe packaging/windows/install.ps1 packaging/windows/install.cmd "$D/"
    for f in "${DOCS[@]}"; do [ -f "$f" ] && cp "$f" "$D/"; done
    printf '%s\r\n' "First Aid Cat $VER for Windows 10 and 11 (x86-64)" "" \
        "  install.cmd               install for the current user: Start menu, widget at sign-in" \
        "  install.cmd -NoWidget     the same without the widget at sign-in" \
        "  install.cmd -Uninstall    remove (or use Settings > Apps)" \
        "  aptechka.exe --help       command line" "" \
        "The program is not code-signed yet, so SmartScreen may warn about an unknown publisher." \
        "Project page, documentation and source: $URL" > "$D/README.txt"
    python3 - "$N" <<'EOF'
import os, sys, zipfile
n = sys.argv[1]
with zipfile.ZipFile(f"dist/{n}.zip", "w", zipfile.ZIP_DEFLATED) as z:
    for f in sorted(os.listdir(f"dist/{n}")):
        z.write(f"dist/{n}/{f}", f"{n}/{f}")
EOF
    echo "dist/$N.zip"
    ;;
macos)
    ARCH=$(uname -m); [ "$ARCH" = arm64 ] || ARCH=x86_64
    N="first-aid-cat-$VER-macos-$ARCH"
    ditto -c -k --keepParent "macos/build/First Aid Cat.app" "dist/$N.zip"
    echo "dist/$N.zip"
    ;;
*)
    echo "usage: packaging/dist.sh linux|windows|macos" >&2; exit 2 ;;
esac
