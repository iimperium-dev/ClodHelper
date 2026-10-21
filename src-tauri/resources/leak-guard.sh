#!/bin/sh
# «Защита от утечек» Clod Helper (macOS): брандмауэр pf запрещает ПРЯМОЙ выход
# к сетям сервисов — Claude (Anthropic) и Telegram. Мост ходит к ним только
# через ваш прокси, поэтому его это не задевает; любая программа, которая
# попробует мимо моста, получит мгновенный отказ, а не ваш IP.
#
# Правила живут в своём якоре com.apple/250.ClodHelper (основной набор правил
# системы не трогаем). pf включается с токеном (pfctl -E) — снятие защиты
# отдаёт только свой токен. Служба (LaunchDaemon) возвращает защиту после
# перезагрузки (якоря pf не сохраняются).
#
# Использование: sh leak-guard.sh Status|Enable|Disable|Restore [claude,telegram]
# Enable/Disable/Restore требуют root. Результат — одна строка JSON в stdout.

ACTION="${1:-Status}"
SERVICES="$2"
ANCHOR="com.apple/250.ClodHelper"
DIR="/Library/Application Support/ClodHelper"
STATE="$DIR/leak-guard.state"
TOKEN="$DIR/leak-guard.token"
RULES="$DIR/leak-guard.pf"
SELF="$DIR/leak-guard.sh"
LABEL="ru.iimperium.clodhelper.leakguard"
DAEMON="/Library/LaunchDaemons/$LABEL.plist"

fail() { printf '{"ok":false,"status":null,"error":"%s"}\n' "$1"; exit 1; }

services() { [ -f "$STATE" ] && sed -n 's/^ClodHelper 1 \(.*\)$/\1/p' "$STATE"; }

status_json() {
    s=$(services)
    on=false; [ -n "$s" ] && on=true
    restore=false; [ -f "$DAEMON" ] && restore=true
    printf '{"ok":true,"status":{"enabled":%s,"services":"%s","auto_restore":%s},"error":null}\n' "$on" "$s" "$restore"
}

# Сети сервисов (официальные диапазоны; IPv4 и IPv6).
nets_for() {
    out=""
    case ",$1," in *,claude,*) out="$out 160.79.104.0/21, 2607:6bc0::/48," ;; esac
    case ",$1," in *,telegram,*) out="$out 149.154.160.0/20, 91.108.0.0/16, 91.105.192.0/23, 95.161.64.0/20, 185.76.151.0/24, 2001:b28:f23d::/48, 2001:b28:f23f::/48, 2001:67c:4e8::/48, 2a0a:f280::/32," ;; esac
    printf '%s' "$out" | sed 's/,$//'
}

apply() {
    nets=$(nets_for "$1")
    [ -n "$nets" ] || return 1
    printf 'table <clod_guard> persist {%s }\nblock return out quick to <clod_guard>\n' "$nets" > "$RULES" || return 1
    /bin/chmod 644 "$RULES"
    # Якорь com.apple/* подключён в системном /etc/pf.conf; если основной
    # набор правил ещё не загружен (pf выключен с загрузки) — загрузить его.
    /sbin/pfctl -sr 2>/dev/null | grep -q 'anchor "com.apple/\*"' || /sbin/pfctl -q -f /etc/pf.conf >/dev/null 2>&1
    /sbin/pfctl -a "$ANCHOR" -f "$RULES" >/dev/null 2>&1 || return 1
    # Включить pf со своим токеном (не мешает другим, кто тоже его включил).
    tok=$(/sbin/pfctl -E 2>&1 | sed -n 's/^Token : //p')
    [ -n "$tok" ] && printf '%s' "$tok" > "$TOKEN"
    return 0
}

remove_rules() {
    /sbin/pfctl -a "$ANCHOR" -F all >/dev/null 2>&1
    [ -f "$TOKEN" ] && /sbin/pfctl -X "$(cat "$TOKEN")" >/dev/null 2>&1
    /bin/rm -f "$TOKEN" "$RULES"
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
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
PLIST
    /usr/sbin/chown root:wheel "$DAEMON.tmp" && /bin/chmod 644 "$DAEMON.tmp" || return 1
    /bin/mv -f "$DAEMON.tmp" "$DAEMON" || return 1
    /bin/launchctl bootout "system/$LABEL" >/dev/null 2>&1
    /bin/launchctl bootstrap system "$DAEMON" >/dev/null 2>&1 || true
}

case "$ACTION" in
    Status) status_json; exit 0 ;;
    Restore)
        [ "$(/usr/bin/id -u)" = 0 ] || exit 0
        s=$(services); [ -n "$s" ] && apply "$s"
        exit 0 ;;
    Enable|Disable) [ "$(/usr/bin/id -u)" = 0 ] || fail "Нужны права администратора." ;;
    *) fail "Неизвестная операция." ;;
esac

if [ "$ACTION" = Enable ]; then
    case "$SERVICES" in
        claude|telegram|claude,telegram|telegram,claude) ;;
        *) fail "Не выбраны сервисы для защиты." ;;
    esac
    /bin/mkdir -p "$DIR" && /usr/sbin/chown root:wheel "$DIR" && /bin/chmod 755 "$DIR" || fail "Нет доступа к папке ClodHelper."
    apply "$SERVICES" || fail "Не удалось включить правила брандмауэра."
    printf 'ClodHelper 1 %s' "$SERVICES" > "$STATE" && /bin/chmod 644 "$STATE"
    install_daemon
    status_json; exit 0
fi

# Disable
/bin/launchctl bootout "system/$LABEL" >/dev/null 2>&1
/bin/rm -f "$DAEMON" "$DAEMON.tmp" "$STATE"
remove_rules
/bin/rm -f "$SELF"
status_json
exit 0
