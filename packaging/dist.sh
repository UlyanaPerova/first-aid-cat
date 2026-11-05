#!/bin/bash
# Release archives and installers from an already built binary — the same locally and in CI.
#   packaging/dist.sh linux    — engine/target/release/aptechka                         → dist/first-aid-cat-VER-linux-x86_64.tar.gz
#   packaging/dist.sh windows  — engine/target/x86_64-pc-windows-gnu/release/aptechka.exe → dist/first-aid-cat-VER-windows-x86_64.zip
#   packaging/dist.sh macos    — "macos/build/First Aid Cat.app"                        → dist/first-aid-cat-VER-macos-ARCH.zip
#   packaging/dist.sh deb      — engine/target/release/aptechka                         → dist/first-aid-cat_VER_amd64.deb
#   packaging/dist.sh rpm      — the same binary (needs rpmbuild)                       → dist/first-aid-cat-VER-1.x86_64.rpm
#   packaging/dist.sh appimage — the same binary (downloads a pinned appimagetool)      → dist/first-aid-cat-VER-linux-x86_64.AppImage
#   packaging/dist.sh dmg      — "macos/build/First Aid Cat.app"                        → dist/first-aid-cat-VER-macos-ARCH.dmg
# The Windows setup .exe is built by Inno Setup (packaging/windows/first-aid-cat.iss), see release.yml.
# The version is taken from engine/Cargo.toml.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' engine/Cargo.toml | head -1)
DOCS=(LICENSE NOTICE LICENSE-ART.md THIRD_PARTY_NOTICES.md PRIVACY.md)
URL="https://github.com/UlyanaPerova/first-aid-cat"

# the documents live in the repository root (public repo) or in release/ (dev repo)
doc_path(){ if [ -f "$1" ]; then echo "$1"; elif [ -f "release/$1" ]; then echo "release/$1"; fi; }

# Desktop Entry word for Exec=: double quotes, \ " ` $ escaped, % doubled, then every \ doubled again
dq_exec(){
    local s=$1
    s=${s//\\/\\\\}; s=${s//\"/\\\"}; s=${s//\`/\\\`}; s=${s//\$/\\\$}; s=${s//%/%%}
    s=${s//\\/\\\\}
    printf '"%s"' "$s"
}

# The menu entry shared by deb, rpm and AppImage (the command is on PATH: Exec=aptechka).
# Same actions as in packaging/linux/install.sh.
write_desktop(){
    local TEXT_SCRIPT
    TEXT_SCRIPT=$(cat <<'EOS'
x-terminal-emulator -e sh -c '"$0" --diag; read' "$0" || gnome-terminal -- sh -c '"$0" --diag; read' "$0"
EOS
)
    cat > "$1" <<DESKTOP
[Desktop Entry]
Type=Application
Name=First Aid Cat
Name[ru]=Аптечка
Comment=Network, VPN, AI agents, graphics, sound, system — check and fix
Comment[ru]=Сеть, VPN, ИИ-агенты, графика, звук, система — проверить и вылечить
Exec=aptechka
Icon=io.github.aptechka
Terminal=false
Categories=Utility;
Keywords=vpn;network;proxy;claude;codex;доктор;сеть;звук;
StartupWMClass=io.github.aptechka
Actions=widget;text;

[Desktop Action widget]
Name=Desktop widget
Name[ru]=Виджет на рабочий стол
Exec=aptechka --widget

[Desktop Action text]
Name=Text mode (if graphics are broken)
Name[ru]=Текстовый режим (если сломалась графика)
Exec=sh -c $(dq_exec "$TEXT_SCRIPT") aptechka
DESKTOP
}

# the documents into directory $1
copy_docs(){
    local f p
    mkdir -p "$1"
    for f in "${DOCS[@]}"; do p=$(doc_path "$f"); if [ -n "$p" ]; then install -m 644 "$p" "$1/$f"; fi; done
    cat > "$1/copyright" <<EOF
First Aid Cat $VER
Project page: $URL

The program is licensed under the Apache License 2.0 (the full text is in LICENSE
next to this file). The cat artwork has its own terms (LICENSE-ART.md); bundled
fonts and third-party components are listed in NOTICE and THIRD_PARTY_NOTICES.md.
EOF
    chmod 644 "$1/copyright"
}

# the /usr tree of the Linux packages in directory $1 (the same for deb and rpm)
stage_linux(){
    local R=$1
    rm -rf "$R"
    install -d -m 755 "$R/usr/bin" "$R/usr/share/applications" "$R/usr/share/icons/hicolor/256x256/apps"
    install -m 755 engine/target/release/aptechka "$R/usr/bin/aptechka"
    install -m 644 assets/aptechka.png "$R/usr/share/icons/hicolor/256x256/apps/io.github.aptechka.png"
    write_desktop "$R/usr/share/applications/io.github.aptechka.desktop"
    chmod 644 "$R/usr/share/applications/io.github.aptechka.desktop"
    copy_docs "$R/usr/share/doc/first-aid-cat"
    find "$R" -type d -exec chmod 755 {} +
}

# the icon must be what its install path says
check_icon(){
    python3 -I - <<'EOF'
import struct, sys
d = open("assets/aptechka.png", "rb").read(24)
w, h = struct.unpack(">II", d[16:24])
if (w, h) != (256, 256):
    sys.exit(f"assets/aptechka.png is {w}x{h}, the packages expect 256x256")
EOF
}

APPIMAGETOOL_VER=1.9.0
APPIMAGETOOL_SHA256=46fdd785094c7f6e545b61afcfb0f3d98d8eab243f644b4b17698c01d06083d1
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VER/appimagetool-x86_64.AppImage"
# the AppImage runtime (the part that mounts the image) is a rolling "continuous" download, so it is pinned by hash too
RUNTIME_SHA256=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64"

mkdir -p dist

case "${1:-}" in
linux)
    N="first-aid-cat-$VER-linux-x86_64"; D="dist/$N"
    rm -rf "$D"; mkdir -p "$D"
    cp engine/target/release/aptechka packaging/linux/install.sh assets/aptechka.png "$D/"
    for f in "${DOCS[@]}"; do p=$(doc_path "$f"); if [ -n "$p" ]; then cp "$p" "$D/"; fi; done
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
    for f in "${DOCS[@]}"; do p=$(doc_path "$f"); if [ -n "$p" ]; then cp "$p" "$D/"; fi; done
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
deb)
    check_icon
    R="dist/deb-root"; stage_linux "$R"
    install -d -m 755 "$R/DEBIAN"
    SIZE=$(du -sk --apparent-size "$R/usr" | cut -f1)
    cat > "$R/DEBIAN/control" <<EOF
Package: first-aid-cat
Version: $VER
Section: utils
Priority: optional
Architecture: amd64
Installed-Size: $SIZE
Maintainer: UlyanaPerova <193013874+UlyanaPerova@users.noreply.github.com>
Depends: libc6 (>= 2.35), libgcc-s1
Recommends: libx11-6, libx11-xcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0
Homepage: $URL
Description: Computer health check with a doctor cat
 Network, VPN, AI agents, graphics, sound and the system: checks what is wrong
 and fixes it. Window, desktop widget and command line (aptechka --diag).
EOF
    for s in postinst postrm; do
        cat > "$R/DEBIAN/$s" <<'EOF'
#!/bin/sh
set -e
update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || true
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || true
exit 0
EOF
        chmod 755 "$R/DEBIAN/$s"
    done
    dpkg-deb --root-owner-group --build "$R" "dist/first-aid-cat_${VER}_amd64.deb" >/dev/null
    rm -rf "$R"
    echo "dist/first-aid-cat_${VER}_amd64.deb"
    ;;
rpm)
    check_icon
    command -v rpmbuild >/dev/null || { echo "rpmbuild not found (apt install rpm)" >&2; exit 1; }
    T="$ROOT/dist/rpm-top"; rm -rf "$T"; mkdir -p "$T/BUILD" "$T/RPMS" "$T/SPECS"
    stage_linux "$T/stage"
    cat > "$T/SPECS/first-aid-cat.spec" <<EOF
Name:           first-aid-cat
Version:        $VER
Release:        1
Summary:        Computer health check with a doctor cat
License:        Apache-2.0
URL:            $URL
BuildArch:      x86_64
Packager:       UlyanaPerova <193013874+UlyanaPerova@users.noreply.github.com>
Recommends:     libX11, libX11-xcb, libXcursor, libXi, libxkbcommon, libxkbcommon-x11
# the binary is built elsewhere: no debuginfo, no stripping, no build-id links
%global debug_package %{nil}
%global __os_install_post %{nil}
%global _build_id_links none

%description
Network, VPN, AI agents, graphics, sound and the system: checks what is wrong
and fixes it. Window, desktop widget and command line (aptechka --diag).

%install
mkdir -p %{buildroot}
cp -a $T/stage/usr %{buildroot}/usr

%files
/usr/bin/aptechka
/usr/share/applications/io.github.aptechka.desktop
/usr/share/icons/hicolor/256x256/apps/io.github.aptechka.png
/usr/share/doc/first-aid-cat

%post
update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :

%postun
update-desktop-database -q /usr/share/applications >/dev/null 2>&1 || :
gtk-update-icon-cache -q -t -f /usr/share/icons/hicolor >/dev/null 2>&1 || :
EOF
    rpmbuild -bb --quiet --define "_topdir $T" "$T/SPECS/first-aid-cat.spec"
    cp "$T/RPMS/x86_64/first-aid-cat-$VER-1.x86_64.rpm" dist/
    rm -rf "$T"
    echo "dist/first-aid-cat-$VER-1.x86_64.rpm"
    ;;
appimage)
    check_icon
    TOOLS="dist/.tools"; mkdir -p "$TOOLS"
    AT="$TOOLS/appimagetool-$APPIMAGETOOL_VER-x86_64.AppImage"
    if [ ! -f "$AT" ] || ! echo "$APPIMAGETOOL_SHA256  $AT" | sha256sum -c --status; then
        curl -fsSL --retry 3 -o "$AT" "$APPIMAGETOOL_URL"
    fi
    echo "$APPIMAGETOOL_SHA256  $AT" | sha256sum -c --status || { echo "appimagetool: wrong sha256" >&2; rm -f "$AT"; exit 1; }
    chmod 755 "$AT"
    RT="$TOOLS/runtime-x86_64"
    if [ ! -f "$RT" ] || ! echo "$RUNTIME_SHA256  $RT" | sha256sum -c --status; then
        curl -fsSL --retry 3 -o "$RT" "$RUNTIME_URL"
    fi
    echo "$RUNTIME_SHA256  $RT" | sha256sum -c --status || { echo "AppImage runtime: wrong sha256 (the rolling download changed: check it and update RUNTIME_SHA256)" >&2; rm -f "$RT"; exit 1; }
    A="dist/AppDir"; rm -rf "$A"
    install -d -m 755 "$A/usr/bin" "$A/usr/share/applications" "$A/usr/share/icons/hicolor/256x256/apps"
    install -m 755 engine/target/release/aptechka "$A/usr/bin/aptechka"
    install -m 644 assets/aptechka.png "$A/io.github.aptechka.png"
    install -m 644 assets/aptechka.png "$A/usr/share/icons/hicolor/256x256/apps/io.github.aptechka.png"
    ln -s io.github.aptechka.png "$A/.DirIcon"
    write_desktop "$A/io.github.aptechka.desktop"
    chmod 644 "$A/io.github.aptechka.desktop"
    cp "$A/io.github.aptechka.desktop" "$A/usr/share/applications/"
    copy_docs "$A/usr/share/doc/first-aid-cat"
    cat > "$A/AppRun" <<'EOF'
#!/bin/sh
HERE="$(dirname "$(readlink -f "$0")")"
export PATH="$HERE/usr/bin:$PATH"
exec "$HERE/usr/bin/aptechka" "$@"
EOF
    chmod 755 "$A/AppRun"
    OUT="dist/first-aid-cat-$VER-linux-x86_64.AppImage"
    # --appimage-extract-and-run: the tool itself needs no FUSE (CI runners and containers have none)
    ARCH=x86_64 "$AT" --appimage-extract-and-run --no-appstream --runtime-file "$RT" "$A" "$OUT" >/dev/null
    rm -rf "$A"
    echo "$OUT"
    ;;
dmg)
    ARCH=$(uname -m); [ "$ARCH" = arm64 ] || ARCH=x86_64
    N="first-aid-cat-$VER-macos-$ARCH"
    S="dist/dmg-root"; rm -rf "$S"; mkdir -p "$S"
    ditto "macos/build/First Aid Cat.app" "$S/First Aid Cat.app"
    ln -s /Applications "$S/Applications"
    rm -f "dist/$N.dmg"
    hdiutil create -volname "First Aid Cat" -srcfolder "$S" -fs HFS+ -format UDZO -ov "dist/$N.dmg" >/dev/null
    rm -rf "$S"
    echo "dist/$N.dmg"
    ;;
*)
    echo "usage: packaging/dist.sh linux|windows|macos|deb|rpm|appimage|dmg" >&2; exit 2 ;;
esac
