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
# Использование: sh claude-proxy-policy.sh Status|Enable|Disable
# Enable/Disable требуют root. Результат — одна строка JSON в stdout.

ACTION="${1:-Status}"
POLICY="/Library/Managed Preferences/com.anthropic.claudefordesktop.plist"
OWNER_DIR="/Library/Application Support/ClodHelper"
OWNER="$OWNER_DIR/claude-proxy-policy.state"
URL="http://127.0.0.1:8890"
PB=/usr/libexec/PlistBuddy

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
    printf '{"ok":true,"status":{"configured":%s,"managed":%s,"conflict":%s,"proxy_url":%s,"pac_configured":%s,"notice":%s},"error":null}\n' \
        "$configured" "$managed" "$c" "$proxy" "$pac" "$n"
}

conflict_now() { status_json | grep -q '"conflict":"'; }

# cfprefsd кэширует управляемые настройки — сбрасываем кэш после записи.
flush_prefs() { /usr/bin/killall cfprefsd >/dev/null 2>&1 || true; }

case "$ACTION" in
    Status)
        status_json
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
    /bin/mkdir -p "/Library/Managed Preferences"
    if has_key egressProxyUrl; then
        "$PB" -c "Set :egressProxyUrl $URL" "$POLICY" >/dev/null 2>&1
    else
        "$PB" -c "Add :egressProxyUrl string $URL" "$POLICY" >/dev/null 2>&1
    fi
    /usr/sbin/chown root:wheel "$POLICY" 2>/dev/null
    /bin/chmod 644 "$POLICY" 2>/dev/null
    flush_prefs
    if [ "$(fixed_state)" != ours ] || conflict_now; then
        fail "Не удалось подтвердить применение фиксированного прокси Claude. Резервная копия сохранена."
    fi
    NOTICE="Полностью закройте Claude Desktop (Cmd+Q) и откройте снова, чтобы применить настройку. Если после перезагрузки macOS она пропадёт, нажмите «Настроить Claude» ещё раз. Эта настройка сама по себе не блокирует обход прокси."
    status_json
    exit 0
fi

# Disable
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
