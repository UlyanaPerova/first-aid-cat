#!/bin/bash
# Установка Аптечки (Rust) для текущего пользователя — без пароля и без пакетов.
#   install.sh               — поставить: программа, ярлык с котом в меню и доке, автозапуск виджета
#   install.sh --no-widget   — без автозапуска виджета (уже стоящий автозапуск убирается)
#   install.sh --desktop     — ещё и ярлык на рабочий стол (если он уже есть — обновится и без флага)
#   install.sh --uninstall   — убрать всё, что поставили (ярлыки, значок, виджет; прежний ярлык вернуть из .bak)
# Программу берёт рядом со скриптом (aptechka из архива релиза) или из сборки engine/target/release.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." 2>/dev/null && pwd || echo "$HERE")"
BIN_DIR="$HOME/.local/bin"; APPS="$HOME/.local/share/applications"; AUTO="$HOME/.config/autostart"
ICONS="$HOME/.local/share/icons/hicolor/256x256/apps"; ID=io.github.aptechka
DESK="$APPS/$ID.desktop"; WDESK="$AUTO/$ID-widget.desktop"
# сообщения — на языке системы, как и сама программа: log "по-русски" "in English"
case "${LC_ALL:-${LC_MESSAGES:-${LANG:-}}}" in ru*|be*|kk*|ky*) RU=1 ;; *) RU=0 ;; esac
log(){ if [ $RU = 1 ]; then echo "[установка] $1"; else echo "[install] ${2:-$1}"; fi; }

DO_UNINSTALL=0; DO_DESKTOP=0; NO_WIDGET=0
for a in "$@"; do
    case "$a" in
        --uninstall) DO_UNINSTALL=1 ;;
        --desktop) DO_DESKTOP=1 ;;
        --no-widget) NO_WIDGET=1 ;;
        *) log "неизвестный параметр: $a" "unknown option: $a"; exit 2 ;;
    esac
done

# Слово для Exec= по Desktop Entry Spec: в двойных кавычках, внутри экранируем \ " ` $,
# % удваиваем (иначе это код поля), потом все \ удваиваем ещё раз — это правило для строк в .desktop.
dq_exec(){
    local s=$1
    s=${s//\\/\\\\}; s=${s//\"/\\\"}; s=${s//\`/\\\`}; s=${s//\$/\\\$}; s=${s//%/%%}
    s=${s//\\/\\\\}
    printf '"%s"' "$s"
}

# Остановить запущенный виджет. Ищем по /proc процессы этого пользователя, у которых argv[0] — aptechka
# и среди аргументов есть точно --widget: окно Аптечки и чужие процессы с похожим именем не заденем.
stop_widget(){
    local d pid a first is_ours has_w
    for d in /proc/[0-9]*; do
        pid=${d#/proc/}
        [ -O "$d" ] || continue
        is_ours=0; has_w=0; first=1
        while IFS= read -r -d '' a; do
            if [ $first = 1 ]; then
                [ "$(basename -- "$a")" = aptechka ] && is_ours=1
                first=0
            elif [ "$a" = "--widget" ]; then
                has_w=1
            fi
        done 2>/dev/null < "$d/cmdline" || true
        if [ $is_ours = 1 ] && [ $has_w = 1 ] && [ "$pid" != "$$" ]; then
            if kill "$pid" 2>/dev/null; then log "виджет остановлен (PID $pid)" "widget stopped (PID $pid)"; fi
        fi
    done
}

DESKTOP_DIR="$(xdg-user-dir DESKTOP 2>/dev/null || echo "$HOME/Desktop")"

if [ $DO_UNINSTALL = 1 ]; then
    stop_widget
    rm -f "$WDESK" "$ICONS/$ID.png"
    # ярлык на рабочем столе убираем, только если он наш (по значку)
    if [ -f "$DESKTOP_DIR/Аптечка.desktop" ] && grep -q "^Icon=$ID\$" "$DESKTOP_DIR/Аптечка.desktop"; then
        rm -f "$DESKTOP_DIR/Аптечка.desktop"; log "убрала ярлык с рабочего стола" "removed the desktop shortcut"
    fi
    if [ -f "$DESK.bak" ]; then mv -f "$DESK.bak" "$DESK"; log "вернула прежний ярлык" "restored the previous menu entry"; else rm -f "$DESK"; fi
    rm -f "$BIN_DIR/aptechka"
    gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true
    update-desktop-database "$APPS" >/dev/null 2>&1 || true
    log "Аптечка убрана (настройки в ~/.config/aptechka остались)" "First Aid Cat removed (settings in ~/.config/aptechka are kept)"; exit 0
fi

BIN="$HERE/aptechka"; [ -x "$BIN" ] || BIN="$ROOT/engine/target/release/aptechka"
[ -x "$BIN" ] || { log "не нашла программу: собери её (cd engine && cargo build --release) или положи aptechka рядом" "program not found: build it (cd engine && cargo build --release) or put aptechka next to this script"; exit 1; }
ICON="$HERE/aptechka.png"; [ -f "$ICON" ] || ICON="$ROOT/assets/aptechka.png"
mkdir -p "$BIN_DIR" "$APPS" "$AUTO" "$ICONS"

install -m 755 "$BIN" "$BIN_DIR/aptechka"
install -m 644 "$ICON" "$ICONS/$ID.png"
[ -f "$HOME/.local/share/icons/hicolor/index.theme" ] || cp /usr/share/icons/hicolor/index.theme "$HOME/.local/share/icons/hicolor/" 2>/dev/null || true
gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

[ -f "$DESK" ] && [ ! -f "$DESK.bak" ] && cp "$DESK" "$DESK.bak" && log "прежний ярлык сохранён: $DESK.bak" "previous menu entry saved: $DESK.bak"
# Текстовый режим: путь к программе передаётся отдельным аргументом ($0), поэтому пробелы и кавычки в нём безопасны
TEXT_SCRIPT=$(cat <<'EOS'
x-terminal-emulator -e sh -c '"$0" --diag; read' "$0" || gnome-terminal -- sh -c '"$0" --diag; read' "$0"
EOS
)
cat > "$DESK" <<DESKTOP
[Desktop Entry]
Type=Application
Name=First Aid Cat
Name[ru]=Аптечка
Comment=Network, VPN, AI agents, graphics, sound, system — check and fix
Comment[ru]=Сеть, VPN, ИИ-агенты, графика, звук, система — проверить и вылечить
Exec=$(dq_exec "$BIN_DIR/aptechka")
Icon=$ID
Terminal=false
Categories=Utility;
Keywords=vpn;network;proxy;claude;codex;доктор;сеть;звук;
StartupWMClass=$ID
Actions=widget;text;

[Desktop Action widget]
Name=Desktop widget
Name[ru]=Виджет на рабочий стол
Exec=$(dq_exec "$BIN_DIR/aptechka") --widget

[Desktop Action text]
Name=Text mode (if graphics are broken)
Name[ru]=Текстовый режим (если сломалась графика)
Exec=sh -c $(dq_exec "$TEXT_SCRIPT") $(dq_exec "$BIN_DIR/aptechka")
DESKTOP
update-desktop-database "$APPS" >/dev/null 2>&1 || true
log "ярлык: $DESK" "menu entry: $DESK"
# ярлык на рабочем столе: обновить, если он уже есть (или поставить с --desktop)
if [ -d "$DESKTOP_DIR" ] && { [ -f "$DESKTOP_DIR/Аптечка.desktop" ] || [ $DO_DESKTOP = 1 ]; }; then
    install -m 755 "$DESK" "$DESKTOP_DIR/Аптечка.desktop"
    gio set "$DESKTOP_DIR/Аптечка.desktop" metadata::trusted true 2>/dev/null || true
    log "ярлык на рабочем столе обновлён" "desktop shortcut updated"
fi

if [ $NO_WIDGET = 1 ]; then
    # --no-widget убирает и уже стоящий автозапуск, иначе флаг ничего бы не значил
    if [ -f "$WDESK" ]; then rm -f "$WDESK"; log "автозапуск виджета убран" "widget autostart removed"; fi
else
    cat > "$WDESK" <<DESKTOP
[Desktop Entry]
Type=Application
Name=First Aid Cat — widget
Name[ru]=Аптечка — виджет
Comment=The doctor cat and the essentials of your system on the desktop
Comment[ru]=Кот-доктор и главное о системе на рабочем столе
Exec=$(dq_exec "$BIN_DIR/aptechka") --widget
Icon=$ID
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=5
NoDisplay=true
DESKTOP
    log "виджет будет запускаться при входе: $WDESK" "the widget will start at login: $WDESK"
fi

# в док, если его там ещё нет (GNOME)
FAV="$(gsettings get org.gnome.shell favorite-apps 2>/dev/null || true)"
case "$FAV" in
    *"$ID.desktop"*|"") ;;
    *) gsettings set org.gnome.shell favorite-apps "$(echo "$FAV" | sed "s/]$/, '$ID.desktop']/")" 2>/dev/null && log "добавила в док" "added to the dock" ;;
esac
log "готово. Запуск: aptechka (окно), aptechka --widget (виджет), aptechka --diag (в терминале)" "done. Run: aptechka (window), aptechka --widget (widget), aptechka --diag (terminal)"
