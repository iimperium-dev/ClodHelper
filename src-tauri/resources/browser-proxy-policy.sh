#!/bin/sh
# Закрепить прокси в Chromium-браузере политикой macOS (Clod Helper).
#
# Управляемые настройки /Library/Managed Preferences/<домен>.plist главнее
# параметров запуска и расширений (FoxyProxy, SwitchyOmega): браузер идёт
# через мост, даже открытый из Dock, а WebRTC не раскрывает настоящий IP.
#   ProxySettings      = { ProxyMode = fixed_servers; ProxyServer = <вход моста> }
#   WebRtcIPHandling   = disable_non_proxied_udp
#
# Запись владения (что ключей не было) — в root-папке ClodHelper; чужие
# значения этих ключей не перезаписываются (конфликт). Enable ставит службу
# (LaunchDaemon), которая возвращает настройку, если macOS сотрёт Managed
# Preferences; Disable убирает и ключи, и службу.
#
# Использование: sh browser-proxy-policy.sh Status|Enable|Disable|Restore <домен> [сервер]
#   домен  — com.google.Chrome | com.brave.Browser | com.microsoft.Edge | org.chromium.Chromium
#   сервер — 127.0.0.1:8890 (Enable)
# Enable/Disable/Restore требуют root. Результат — одна строка JSON в stdout.

ACTION="${1:-Status}"
DOMAIN="$2"
SERVER="$3"
case "$DOMAIN" in
    com.google.Chrome|com.brave.Browser|com.microsoft.Edge|org.chromium.Chromium) ;;
    *) printf '{"ok":false,"status":null,"error":"Неизвестный браузер"}\n'; exit 1 ;;
esac
POLICY="/Library/Managed Preferences/$DOMAIN.plist"
OWNER_DIR="/Library/Application Support/ClodHelper"
OWNER="$OWNER_DIR/browser-proxy-$DOMAIN.state"
SELF="$OWNER_DIR/browser-proxy-policy.sh"
LABEL="ru.iimperium.clodhelper.browserpolicy.$DOMAIN"
DAEMON="/Library/LaunchDaemons/$LABEL.plist"
PB=/usr/libexec/PlistBuddy

fail() { printf '{"ok":false,"status":null,"error":"%s"}\n' "$1"; exit 1; }
get() { "$PB" -c "Print :$1" "$POLICY" 2>/dev/null; }
has() { [ -f "$POLICY" ] && "$PB" -c "Print :$1" "$POLICY" >/dev/null 2>&1; }

# Записанный нами сервер или пусто (записи нет).
owner_server() { [ -e "$OWNER" ] && sed -n 's/^ClodHelper 1 \(.*\)$/\1/p' "$OWNER"; }

# ours — оба ключа наши; absent — ни одного; foreign — что-то чужое.
state() {
    srv=$(owner_server)
    if ! has ProxySettings && ! has WebRtcIPHandling; then echo absent; return; fi
    if [ -n "$srv" ] && [ "$(get ProxySettings:ProxyServer)" = "$srv" ] \
        && [ "$(get ProxySettings:ProxyMode)" = fixed_servers ] \
        && [ "$(get WebRtcIPHandling)" = disable_non_proxied_udp ]; then
        echo ours; return
    fi
    echo foreign
}

NOTICE=""
status_json() {
    st=$(state)
    configured=false; [ "$st" = ours ] && configured=true
    managed=false; [ -n "$(owner_server)" ] && managed=true
    conflict=null
    [ "$st" = foreign ] && conflict='"У браузера уже есть чужая политика прокси — Clod Helper её не меняет."'
    restore=false; [ -f "$DAEMON" ] && [ -f "$SELF" ] && restore=true
    n=null; [ -n "$NOTICE" ] && n="\"$NOTICE\""
    printf '{"ok":true,"status":{"configured":%s,"managed":%s,"conflict":%s,"auto_restore":%s,"server":"%s","notice":%s},"error":null}\n' \
        "$configured" "$managed" "$conflict" "$restore" "$(owner_server)" "$n"
}

write_ours() {
    /bin/mkdir -p "/Library/Managed Preferences"
    "$PB" -c "Delete :ProxySettings" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Add :ProxySettings dict" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Add :ProxySettings:ProxyMode string fixed_servers" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Add :ProxySettings:ProxyServer string $1" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Add :ProxySettings:ProxyBypassList string <local>" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Delete :WebRtcIPHandling" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Add :WebRtcIPHandling string disable_non_proxied_udp" "$POLICY" >/dev/null 2>&1
    /usr/sbin/chown root:wheel "$POLICY" 2>/dev/null
    /bin/chmod 644 "$POLICY" 2>/dev/null
    /usr/bin/killall cfprefsd >/dev/null 2>&1 || true
}

install_daemon() {
    [ -f "$SELF" ] || return 1
    /usr/sbin/chown root:wheel "$SELF" && /bin/chmod 755 "$SELF" || return 1
    cat > "$DAEMON.tmp" <<PLIST || return 1
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$LABEL</string>
	<key>AssociatedBundleIdentifiers</key>
	<array>
		<string>ru.iimperium.clodhelper</string>
	</array>
	<key>ProgramArguments</key>
	<array>
		<string>/bin/sh</string>
		<string>$SELF</string>
		<string>Restore</string>
		<string>$DOMAIN</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>WatchPaths</key>
	<array>
		<string>/Library/Managed Preferences</string>
	</array>
	<key>ThrottleInterval</key>
	<integer>10</integer>
</dict>
</plist>
PLIST
    /usr/sbin/chown root:wheel "$DAEMON.tmp" && /bin/chmod 644 "$DAEMON.tmp" || return 1
    /bin/mv -f "$DAEMON.tmp" "$DAEMON" || return 1
    /bin/launchctl bootout "system/$LABEL" >/dev/null 2>&1
    /bin/launchctl bootstrap system "$DAEMON" >/dev/null 2>&1 || return 1
}

case "$ACTION" in
    Status) status_json; exit 0 ;;
    Restore)
        [ "$(/usr/bin/id -u)" = 0 ] || exit 0
        srv=$(owner_server)
        # Только откат «к пустому» (macOS стёр настройку); чужое — не трогаем.
        [ -n "$srv" ] && [ "$(state)" = absent ] && write_ours "$srv"
        exit 0 ;;
    Enable|Disable)
        [ "$(/usr/bin/id -u)" = 0 ] || fail "Нужны права администратора." ;;
    *) fail "Неизвестная операция." ;;
esac

if [ "$ACTION" = Enable ]; then
    case "$SERVER" in
        127.0.0.1:[0-9]*) ;;
        *) fail "Ожидался вход моста 127.0.0.1:порт." ;;
    esac
    st=$(state)
    [ "$st" = foreign ] && fail "У браузера уже есть чужая политика прокси — Clod Helper её не меняет."
    /bin/mkdir -p "$OWNER_DIR" && /bin/chmod 755 "$OWNER_DIR" || fail "Не удалось создать папку записи владения."
    printf 'ClodHelper 1 %s' "$SERVER" > "$OWNER" && /bin/chmod 644 "$OWNER" || fail "Не удалось сохранить запись владения."
    write_ours "$SERVER"
    [ "$(state)" = ours ] || fail "Не удалось подтвердить политику браузера."
    if install_daemon; then
        NOTICE="Готово. Перезапустите браузер — дальше он всегда идёт через мост."
    else
        NOTICE="Готово. Перезапустите браузер. Автовосстановление не установилось."
    fi
    status_json; exit 0
fi

# Disable
/bin/launchctl bootout "system/$LABEL" >/dev/null 2>&1
/bin/rm -f "$DAEMON" "$DAEMON.tmp"
if [ "$(state)" = ours ]; then
    "$PB" -c "Delete :ProxySettings" "$POLICY" >/dev/null 2>&1
    "$PB" -c "Delete :WebRtcIPHandling" "$POLICY" >/dev/null 2>&1
    if [ "$("$PB" -c "Print" "$POLICY" 2>/dev/null | tr -d ' \n')" = "Dict{}" ]; then
        /bin/rm -f "$POLICY"
    fi
    /usr/bin/killall cfprefsd >/dev/null 2>&1 || true
fi
/bin/rm -f "$OWNER"
# Копию скрипта убираем, только если ей больше не пользуется ни одна служба.
ls /Library/LaunchDaemons/ru.iimperium.clodhelper.browserpolicy.*.plist >/dev/null 2>&1 || /bin/rm -f "$SELF"
NOTICE="Политика снята. Перезапустите браузер."
status_json
exit 0
