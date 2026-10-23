#!/bin/sh
# Подписать собранный Clod Helper.app своим сертификатом (самоподписанным)
# и пересобрать .dmg.
#
# Зачем: без сертификата подпись «на лету» (ad-hoc) у каждой сборки своя, и
# macOS считает обновлённые clod-helper и clod-bridge новыми фоновыми
# программами — «Добавлены фоновые объекты» после каждого обновления.
# С постоянным сертификатом требование подписи — «идентификатор + корневой
# сертификат», оно не меняется от версии к версии, и macOS узнаёт программу.
# Доверия Gatekeeper сертификат не даёт (как и ad-hoc) — только постоянство.
#
# Использование: sh scripts/sign-mac.sh <путь к .app> [<путь к .dmg>]
# Сертификат — из окружения:
#   CLOD_SIGN_P12_BASE64 + CLOD_SIGN_P12_PASSWORD — .p12 в base64 (CI, секреты);
#   либо CLOD_SIGN_P12 (путь к .p12) + CLOD_SIGN_P12_PASSWORD (локально).
# Без сертификата ничего не делает: сборка остаётся с подписью ad-hoc.
set -eu

APP="${1:?путь к .app}"
DMG="${2:-}"
IDENTITY_NAME="Clod Helper Self-Signed"

if [ -z "${CLOD_SIGN_P12_BASE64:-}" ] && [ -z "${CLOD_SIGN_P12:-}" ]; then
    echo "sign-mac: сертификата нет — оставляю подпись ad-hoc"
    exit 0
fi
: "${CLOD_SIGN_P12_PASSWORD:?нужен CLOD_SIGN_P12_PASSWORD}"

WORK=$(mktemp -d)
KC="$WORK/clod-sign.keychain-db"
KC_PASS=$(openssl rand -hex 16)
ORIG_LIST=$(security list-keychains -d user | tr -d '"' | tr '\n' ' ')

cleanup() {
    # Список связок ключей — как был; временная связка удаляется.
    # shellcheck disable=SC2086
    security list-keychains -d user -s $ORIG_LIST >/dev/null 2>&1 || true
    security delete-keychain "$KC" >/dev/null 2>&1 || true
    rm -rf "$WORK"
}
trap cleanup EXIT INT TERM

if [ -n "${CLOD_SIGN_P12_BASE64:-}" ]; then
    printf '%s' "$CLOD_SIGN_P12_BASE64" | base64 -D > "$WORK/sign.p12"
else
    cp "$CLOD_SIGN_P12" "$WORK/sign.p12"
fi

security create-keychain -p "$KC_PASS" "$KC"
security set-keychain-settings "$KC"
security unlock-keychain -p "$KC_PASS" "$KC"
security import "$WORK/sign.p12" -k "$KC" -P "$CLOD_SIGN_P12_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple: -s -k "$KC_PASS" "$KC" >/dev/null
# codesign ищет ключ только в связках из списка поиска.
# shellcheck disable=SC2086
security list-keychains -d user -s $ORIG_LIST "$KC"

# Сертификат не «доверенный» — указываем его отпечатком SHA-1.
ID=$(security find-identity -p codesigning "$KC" | awk -v n="\"$IDENTITY_NAME\"" 'index($0, n) { print $2; exit }')
[ -n "$ID" ] || { echo "sign-mac: в .p12 нет «$IDENTITY_NAME»" >&2; exit 1; }

sign() { codesign --force --timestamp=none --keychain "$KC" -s "$ID" --identifier "$1" "$2"; }

# Сначала вложенный мост (его копирует к себе установка), затем приложение:
# подпись приложения запечатывает уже подписанный мост.
BRIDGE="$APP/Contents/Resources/resources/clod-bridge"
[ -f "$BRIDGE" ] && sign ru.iimperium.clodbridge "$BRIDGE"
sign ru.iimperium.clodhelper "$APP"
codesign --verify --strict "$APP"
codesign -d -r- "$APP" 2>&1 | grep designated

if [ -n "$DMG" ]; then
    # .dmg собран Tauri из приложения с прежней подписью — пересобрать.
    STAGE="$WORK/dmg"
    mkdir -p "$STAGE"
    ditto "$APP" "$STAGE/$(basename "$APP")"
    ln -s /Applications "$STAGE/Applications"
    rm -f "$DMG"
    hdiutil create -quiet -volname "Clod Helper" -srcfolder "$STAGE" -fs HFS+ -format UDZO "$DMG"
    echo "sign-mac: $DMG"
fi
