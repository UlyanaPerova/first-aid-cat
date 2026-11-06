#!/bin/bash
# Сборка «First Aid Cat.app» (по-русски Finder показывает «Аптечка») для macOS: Rust-программа + виджет WidgetKit (Swift) внутри, бесплатная подпись (ad-hoc).
# Нужен Xcode (xcode-select -s /Applications/Xcode.app). Результат: macos/build/First Aid Cat.app
# Запуск: macos/build_app.sh            — только собрать
#         macos/build_app.sh --install  — собрать и поставить в «Программы» (Launchpad, Spotlight, галерея виджетов)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(dirname "$HERE")"
OUT="$HERE/build"; NAME="First Aid Cat"; APP="$OUT/$NAME.app"; EXT="$APP/Contents/PlugIns/AptechkaWidget.appex"
log(){ echo "[build_app] $*"; }
rm -rf "$APP"; mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$EXT/Contents/MacOS" "$EXT/Contents/Resources"

log "Rust…"; (cd "$ROOT/engine" && cargo build --release)
cp "$ROOT/engine/target/release/aptechka" "$APP/Contents/MacOS/aptechka"
cp "$HERE/App-Info.plist" "$APP/Contents/Info.plist"
cp "$ROOT/assets/aptechka.icns" "$APP/Contents/Resources/aptechka.icns"   # значок программы
# название в Finder и Launchpad — на языке системы (LSHasLocalizedDisplayName в App-Info.plist)
mkdir -p "$APP/Contents/Resources/en.lproj" "$APP/Contents/Resources/ru.lproj"
printf '"CFBundleDisplayName" = "First Aid Cat";\n"CFBundleName" = "First Aid Cat";\n' > "$APP/Contents/Resources/en.lproj/InfoPlist.strings"
printf '"CFBundleDisplayName" = "Аптечка";\n"CFBundleName" = "Аптечка";\n' > "$APP/Contents/Resources/ru.lproj/InfoPlist.strings"
# лицензии и уведомления — внутри приложения (Contents/Resources); в публичной копии они в корне, в рабочей — в release/
for f in LICENSE NOTICE LICENSE-ART.md THIRD_PARTY_NOTICES.md; do
    if [ -f "$ROOT/$f" ]; then cp "$ROOT/$f" "$APP/Contents/Resources/$f"; elif [ -f "$ROOT/release/$f" ]; then cp "$ROOT/release/$f" "$APP/Contents/Resources/$f"; fi
done

log "виджет (Swift)…"
SDK="$(xcrun --sdk macosx --show-sdk-path)"
xcrun --sdk macosx swiftc -parse-as-library -application-extension -O \
    -target "$(uname -m)-apple-macos14" -sdk "$SDK" -module-name AptechkaWidget \
    "$HERE/widget/AptechkaWidget.swift" -o "$EXT/Contents/MacOS/AptechkaWidget" \
    -Xlinker -e -Xlinker _NSExtensionMain -framework WidgetKit -framework SwiftUI -framework AppKit
cp "$HERE/widget/Info.plist" "$EXT/Contents/Info.plist"
# кот из картинки Аптечки (виджет собирает SwiftUI, картинка — только кот)
cp "$HERE/widget/cat_ok.png" "$EXT/Contents/Resources/cat_ok.png"

log "подпись (ad-hoc)…"
codesign --force --sign - --entitlements "$HERE/widget/Widget.entitlements" --timestamp=none "$EXT"
codesign --force --sign - --timestamp=none "$APP"
codesign --verify --deep --strict "$APP" && log "готово: $APP"

if [ "${1:-}" = "--install" ]; then
    DEST=/Applications; [ -w "$DEST" ] || DEST="$HOME/Applications"; mkdir -p "$DEST"
    pkill -x aptechka 2>/dev/null || true
    # прежнее имя (до 0.9.0 программа называлась «Аптечка.app») — снять с регистрации и убрать
    OLD="$DEST/Аптечка.app"
    if [ -d "$OLD" ]; then
        /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -u "$OLD" 2>/dev/null || true
        rm -rf "$OLD"
    fi
    rm -rf "$DEST/$NAME.app"; ditto "$APP" "$DEST/$NAME.app"
    # сборку в папке проекта убираем из реестра и удаляем (иначе Finder снова её зарегистрирует),
    # и только потом регистрируем установленную: если удалить после, система снимает регистрацию виджета
    LSR=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
    "$LSR" -u "$APP" 2>/dev/null || true; pluginkit -r "$EXT" 2>/dev/null || true
    rm -rf "$APP"
    "$LSR" -f "$DEST/$NAME.app"
    # lsregister -f снимает регистрацию виджета не сразу, а чуть позже — поэтому добавляем, пока не встанет
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        pluginkit -a "$DEST/$NAME.app/Contents/PlugIns/AptechkaWidget.appex" || true; sleep 1
        pluginkit -m -i io.github.aptechka.widget | grep -q . && break
    done
    pluginkit -m -i io.github.aptechka.widget | grep -q . || log "ВНИМАНИЕ: виджет не зарегистрирован — pluginkit -a \"$DEST/$NAME.app/Contents/PlugIns/AptechkaWidget.appex\""
    log "поставлено: $DEST/$NAME.app"
fi
