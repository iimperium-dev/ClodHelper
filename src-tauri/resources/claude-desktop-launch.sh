#!/bin/sh
# Находит Claude Desktop и запускает его через отдельный вход моста
# (аналог claude-desktop-launch.ps1 для macOS).
#
#   sh claude-desktop-launch.sh --detect-only   → JSON установки или null
#   sh claude-desktop-launch.sh [--show-error]  → проверки и запуск
#
# Перед запуском проверяет: Claude не запущен (иначе macOS просто активирует
# старый процесс и флаг прокси не применится), мост 127.0.0.1:8890 отвечает
# в режиме без DIRECT, штатная политика egressProxyUrl указывает на него.

PORT=8890
URL="http://127.0.0.1:$PORT"
POLICY="/Library/Managed Preferences/com.anthropic.claudefordesktop.plist"
DETECT_ONLY=0
SHOW_ERROR=0
for arg in "$@"; do
    case "$arg" in
        --detect-only) DETECT_ONLY=1 ;;
        --show-error) SHOW_ERROR=1 ;;
    esac
done

find_claude() {
    for app in "/Applications/Claude.app" "$HOME/Applications/Claude.app"; do
        if [ -x "$app/Contents/MacOS/Claude" ]; then
            echo "$app"
            return
        fi
    done
    # Нестандартное место: ищем по bundle id через Spotlight.
    found=$(/usr/bin/mdfind "kMDItemCFBundleIdentifier == 'com.anthropic.claudefordesktop'" 2>/dev/null | head -n 1)
    if [ -n "$found" ] && [ -x "$found/Contents/MacOS/Claude" ]; then
        echo "$found"
    fi
}

die() {
    echo "$1" >&2
    if [ "$SHOW_ERROR" = 1 ]; then
        /usr/bin/osascript -e "display alert \"Claude Desktop — прокси\" message \"$1\" as warning" >/dev/null 2>&1
    fi
    exit 1
}

APP=$(find_claude)

if [ "$DETECT_ONLY" = 1 ]; then
    if [ -n "$APP" ]; then
        escaped=$(printf '%s' "$APP" | sed 's/\\/\\\\/g; s/"/\\"/g')
        printf '{"exe_path":"%s","app_id":null,"kind":"exe_flag"}\n' "$escaped"
    else
        echo null
    fi
    exit 0
fi

[ -n "$APP" ] || die "Claude Desktop не найден. Установите приложение с claude.ai/download."

if /usr/bin/pgrep -f "$APP/Contents/MacOS/Claude" >/dev/null 2>&1; then
    die "Claude Desktop уже запущен. Полностью закройте его (Cmd+Q или Claude → Quit), затем снова запустите через Clod Helper или ярлык Claude Desktop (proxy). Это нужно, чтобы приложение применило прокси."
fi

health=$(/usr/bin/curl -s --noproxy '*' --max-time 4 "$URL/healthz" 2>/dev/null)
ok=1
for needle in '"service":"clod-bridge"' '"routing_policy":"upstream_only"' \
    '"direct_allowed":false' '"strict_ready":true' '"mode":"all"' \
    "\"listen\":\"127.0.0.1:$PORT\"" "\"port\":$PORT" '"via_direct":0'; do
    case "$health" in
        *"$needle"*) ;;
        *) ok=0 ;;
    esac
done
[ "$ok" = 1 ] || die "Мост Claude на 127.0.0.1:$PORT не подтвердил работу без DIRECT. Откройте Clod Helper → Приложения → Настроить Claude."

PB=/usr/libexec/PlistBuddy
if [ ! -f "$POLICY" ] || "$PB" -c "Print :egressProxyPacUrl" "$POLICY" >/dev/null 2>&1 ||
    [ "$("$PB" -c "Print :egressProxyUrl" "$POLICY" 2>/dev/null)" != "$URL" ]; then
    die "Штатный прокси Claude ещё не настроен или изменён другой программой. Откройте Clod Helper → Приложения → Настроить Claude."
fi

/usr/bin/open -a "$APP" --args "--proxy-server=$URL" || die "macOS не удалось запустить Claude Desktop."
