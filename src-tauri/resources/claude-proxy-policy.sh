#!/bin/sh
# Штатная политика прокси Claude Desktop на macOS (аналог claude-proxy-policy.ps1).
#
# Claude Desktop читает egressProxyUrl из управляемых настроек:
#   /Library/Managed Preferences/com.anthropic.claudefordesktop.plist
# (туда же кладёт значения MDM-профиль). Это маршрут, а не сетевая изоляция.
#
# Запись владения (что было до нас) — в защищённой root-папке:
#   /Library/Application Support/ClodHelper/claude-proxy-policy.state
# Допустимое исходное значение — только «ключа нет» или пустая строка: чужой
# фиксированный прокси или PAC никогда не перезаписываются.
#
# Использование: sh claude-proxy-policy.sh Status|Enable|Disable|Restore
# Enable/Disable/Restore требуют root. Результат — одна строка JSON в stdout.
#
# Автовосстановление: macOS на машинах без MDM иногда стирает Managed
# Preferences (после перезагрузки/обновления). Enable ставит системную службу
# (LaunchDaemon), которая при загрузке и при любом изменении папки управляемых
# настроек запускает этот же скрипт с Restore: если запись владения Helper есть,
# а значение вернулось к исходному (ключа нет / пусто) — ставит наш прокси
# обратно. Чужой прокси, PAC или изменённое вне Helper значение не трогает.
# Копия скрипта для службы лежит в root-папке OWNER_DIR (пользовательский
# процесс подменить её не может); Disable удаляет и службу, и копию.

ACTION="${1:-Status}"
POLICY="/Library/Managed Preferences/com.anthropic.claudefordesktop.plist"
OWNER_DIR="/Library/Application Support/ClodHelper"
OWNER="$OWNER_DIR/claude-proxy-policy.state"
URL="http://127.0.0.1:8890"
PB=/usr/libexec/PlistBuddy
SELF="$OWNER_DIR/claude-proxy-policy.sh"
DAEMON_LABEL="ru.iimperium.clodhelper.claudepolicy"
DAEMON_PLIST="/Library/LaunchDaemons/$DAEMON_LABEL.plist"

fail() {
    printf '{"ok":false,"status":null,"error":"%s"}\n' "$1"
    exit 1
}

has_key() { [ -f "$POLICY" ] && "$PB" -c "Print :$1" "$POLICY" >/dev/null 2>&1; }
get_key() { "$PB" -c "Print :$1" "$POLICY" 2>/dev/null; }

# Текущее значение egressProxyUrl: absent | empty | ours | foreign.
fixed_state() {
    if ! has_key egressProxyUrl; then echo absent; return; fi
    v=$(get_key egressProxyUrl)
    if [ -z "$v" ]; then echo empty
    elif [ "$v" = "$URL" ]; then echo ours
    else echo foreign; fi
}

# Исходное значение из записи владения: absent | empty | none (записи нет).
owner_original() {
    if [ ! -e "$OWNER" ]; then echo none; return; fi
    case "$(cat "$OWNER" 2>/dev/null)" in
        "ClodHelper 1 absent") echo absent ;;
        "ClodHelper 1 empty") echo empty ;;
        *) echo corrupt ;;
    esac
}

NOTICE=""
status_json() {
    fixed=$(fixed_state)
    original=$(owner_original)
    [ "$original" = corrupt ] && fail "Запись владения политикой Claude повреждена; политика не изменена."
    pac=false
    has_key egressProxyPacUrl && pac=true
    conflict=""
    if [ "$pac" = true ]; then
        conflict="У Claude уже задана PAC-политика с приоритетом над фиксированным прокси. Она сохранена без изменений."
    elif [ "$original" != none ] && [ "$fixed" != ours ] && [ "$fixed" != "$original" ]; then
        conflict="Политика прокси Claude изменена вне Helper. Она сохранена без изменений."
    elif [ "$original" = none ] && [ "$fixed" = foreign ]; then
        conflict="У Claude уже задан фиксированный прокси, который не принадлежит Helper. Он сохранён без изменений."
    fi
    configured=false; proxy=null
    if [ "$fixed" = ours ] && [ "$pac" = false ]; then configured=true; fi
    if [ "$fixed" = ours ]; then proxy="\"$URL\""; fi
    managed=false
    [ "$original" != none ] && managed=true
    c=null; [ -n "$conflict" ] && c="\"$conflict\""
    n=null; [ -n "$NOTICE" ] && n="\"$NOTICE\""
    restore=false
    [ -f "$DAEMON_PLIST" ] && [ -f "$SELF" ] && restore=true
    printf '{"ok":true,"status":{"configured":%s,"managed":%s,"conflict":%s,"proxy_url":%s,"pac_configured":%s,"notice":%s,"auto_restore":%s},"error":null}\n' \
        "$configured" "$managed" "$c" "$proxy" "$pac" "$n" "$restore"
}

conflict_now() { status_json | grep -q '"conflict":"'; }

# cfprefsd кэширует управляемые настройки — сбрасываем кэш после записи.
flush_prefs() { /usr/bin/killall cfprefsd >/dev/null 2>&1 || true; }

write_ours() {
    /bin/mkdir -p "/Library/Managed Preferences"
    if has_key egressProxyUrl; then
        "$PB" -c "Set :egressProxyUrl $URL" "$POLICY" >/dev/null 2>&1
    else
        "$PB" -c "Add :egressProxyUrl string $URL" "$POLICY" >/dev/null 2>&1
    fi
    /usr/sbin/chown root:wheel "$POLICY" 2>/dev/null
    /bin/chmod 644 "$POLICY" 2>/dev/null
    flush_prefs
}

# Служба автовосстановления. Копию скрипта в OWNER_DIR кладёт вызывающий
# (claude_policy.rs, в той же root-сессии) — без неё службу не ставим.
install_daemon() {
    [ -f "$SELF" ] || return 1
    /usr/sbin/chown root:wheel "$SELF" && /bin/chmod 755 "$SELF" || return 1
    cat > "$DAEMON_PLIST.tmp" <<PLIST || return 1
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>$DAEMON_LABEL</string>
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
	<key>WatchPaths</key>
	<array>
		<string>/Library/Managed Preferences</string>
	</array>
	<key>ThrottleInterval</key>
	<integer>10</integer>
</dict>
</plist>
PLIST
    /usr/sbin/chown root:wheel "$DAEMON_PLIST.tmp" && /bin/chmod 644 "$DAEMON_PLIST.tmp" || return 1
    /bin/mv -f "$DAEMON_PLIST.tmp" "$DAEMON_PLIST" || return 1
    /bin/launchctl bootout "system/$DAEMON_LABEL" >/dev/null 2>&1
    /bin/launchctl bootstrap system "$DAEMON_PLIST" >/dev/null 2>&1 || return 1
}

remove_daemon() {
    /bin/launchctl bootout "system/$DAEMON_LABEL" >/dev/null 2>&1
    /bin/rm -f "$DAEMON_PLIST" "$DAEMON_PLIST.tmp" "$SELF"
}

case "$ACTION" in
    Status)
        status_json
        exit 0
        ;;
    Restore)
        # Запускает служба: молча, без JSON.
        [ "$(/usr/bin/id -u)" = 0 ] || exit 0
        original=$(owner_original)
        case "$original" in absent|empty) ;; *) exit 0 ;; esac
        has_key egressProxyPacUrl && exit 0
        # Возвращаем ТОЛЬКО откат к исходному (macOS стёр настройку). Чужое
        # значение («foreign») — чьё-то решение, его не трогаем.
        [ "$(fixed_state)" = "$original" ] || exit 0
        write_ours
        exit 0
        ;;
    Enable|Disable)
        [ "$(/usr/bin/id -u)" = 0 ] || fail "Для изменения политики Claude требуются права администратора."
        [ "$(owner_original)" = corrupt ] && fail "Запись владения политикой Claude повреждена; политика не изменена."
        ;;
    *)
        fail "Неизвестная операция с политикой Claude."
        ;;
esac

if [ "$ACTION" = Enable ]; then
    if conflict_now; then
        fail "$(status_json | sed -n 's/.*"conflict":"\([^"]*\)".*/\1/p')"
    fi
    current=$(fixed_state)
    if [ "$(owner_original)" = none ]; then
        # Наше значение без записи владения (запись удалили вручную): исходным
        # считаем «ключа не было» — иначе резервная копия была бы невалидной.
        original=$current
        [ "$original" = ours ] && original=absent
        # Сначала резервная копия, потом политика — сбой записи восстановим.
        /bin/mkdir -p "$OWNER_DIR" && /bin/chmod 755 "$OWNER_DIR" || fail "Не удалось создать папку резервной копии политики Claude."
        printf 'ClodHelper 1 %s' "$original" > "$OWNER" || fail "Не удалось сохранить резервную копию политики Claude. Политика прокси не изменена."
        /bin/chmod 644 "$OWNER"
        [ "$(owner_original)" = "$original" ] || fail "Не удалось подтвердить резервную копию политики Claude. Политика прокси не изменена."
    fi
    [ "$(fixed_state)" = "$current" ] || fail "Политика прокси Claude изменилась во время настройки. Проверьте её перед повторной попыткой."
    write_ours
    if [ "$(fixed_state)" != ours ] || conflict_now; then
        fail "Не удалось подтвердить применение фиксированного прокси Claude. Резервная копия сохранена."
    fi
    if install_daemon; then
        NOTICE="Готово. Перезапустите Claude Desktop (Cmd+Q и открыть снова). Если macOS сотрёт настройку, Clod Helper вернёт её сам."
    else
        NOTICE="Готово. Перезапустите Claude Desktop (Cmd+Q и открыть снова). Автовосстановление не установилось — если настройка пропадёт, нажмите «Подключить» ещё раз."
    fi
    status_json
    exit 0
fi

# Disable: служба больше не должна возвращать наш прокси.
remove_daemon
original=$(owner_original)
if [ "$original" = none ]; then status_json; exit 0; fi
current=$(fixed_state)
changed_externally=false
if [ "$current" = ours ]; then
    "$PB" -c "Delete :egressProxyUrl" "$POLICY" >/dev/null 2>&1
    if [ "$original" = empty ]; then
        # Add без значения создаёт пустую строку.
        "$PB" -c "Add :egressProxyUrl string" "$POLICY" >/dev/null 2>&1
    else
        # Пустой файл управляемых настроек создавали мы — убираем его.
        if [ "$("$PB" -c "Print" "$POLICY" 2>/dev/null | tr -d ' \n')" = "Dict{}" ]; then
            /bin/rm -f "$POLICY"
        fi
    fi
    flush_prefs
    [ "$(fixed_state)" = "$original" ] || fail "Не удалось подтвердить восстановление политики Claude. Резервная копия сохранена."
elif [ "$current" != "$original" ]; then
    # Значение изменено вне Helper и принадлежит тому, кто его записал.
    changed_externally=true
fi
/bin/rm -f "$OWNER" || fail "Не удалось удалить запись владения политикой Claude."
if [ "$changed_externally" = true ]; then
    NOTICE="Политика изменена вне Helper и сохранена без изменений. Запись владения Helper удалена."
else
    NOTICE="Предыдущая политика прокси Claude восстановлена. Перезапустите Claude Desktop, чтобы применить изменение."
fi
status_json
exit 0
