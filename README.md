# Clod Helper

Утилита для установки **Claude Code**, **Claude Desktop** и туннелирования **Cursor IDE** через прокси.
Нужна тем, у кого Anthropic API блокирует напрямую (например, в России).

Работает через локальный мост, который передаёт трафик в HTTP-прокси с автоматической авторизацией. Общий вход `127.0.0.1:8889` поддерживает Smart/Full; отдельный вход Claude `127.0.0.1:8890` всегда отправляет полученные запросы через upstream и не переключается на DIRECT.

**Cursor IDE** подключается тем же способом, что и Claude Desktop: создаётся ярлык «Cursor (proxy)» на рабочем столе, который запускает Cursor с флагом `--proxy-server=http://127.0.0.1:8889`. Обычный ярлык Cursor идёт напрямую. Выбрать Cursor можно в мастере установки (отдельным тумблером, вместе с Claude или без него) либо на Dashboard кнопкой «Настроить».

---

## Скачать

[Последний релиз →](https://github.com/mogylnyy/Clod-Helper-v1/releases)

Файл `Clod_Helper_*_x64-setup.exe` — ~10 МБ, Windows 10/11.

Для Mac — `*_universal.dmg` (Apple Silicon и Intel, macOS 11+), см. раздел [macOS](#macos).

## Установка

1. **Скачайте** `Clod_Helper_*_x64-setup.exe` из релизов.

2. **Перед запуском разблокируйте файл:**
   ПКМ → **Свойства** → внизу галочка **«Разблокировать»** → **ОК**.
   Это снимает «Mark of the Web» — Windows перестаёт считать файл «недоверенным».

3. **Двойной клик.**
   Появится синее окно SmartScreen «Система Windows защитила ваш компьютер» →
   нажмите **Подробнее** → **Выполнить в любом случае**.

4. **Установщик** добавит «Clod Helper» в Пуск и на рабочий стол.

5. **Запустите** Clod Helper и пройдите 5 шагов мастера:
   приветствие → выбор (Code / Desktop / оба) → URL прокси → проверка → установка.

## Что нужно

- **Купленный HTTP-прокси** в формате `http://login:pass@ip:port`.
  Обязательно **не-российский IP** (Anthropic блокирует RU). Берите Германию, Нидерланды, США, Турцию — ~200₽/мес у любого провайдера.

  👉 Рекомендую **[proxy6.net](https://proxy6.net/?r=692907)** — пользуюсь сам, недорого, страны на выбор. Если купите по этой реферальной ссылке — спасибо, поддержите развитие проекта.

- **Node.js LTS** — нужен Claude Code. Если нет, мастер откроет страницу скачивания.
- Мост `clod-bridge.exe` входит в установщик; Python ему не требуется.

После установки Claude Code запускается командой `claude` в любом терминале. Для Claude Desktop включите штатную настройку в разделе **«Приложения → Настроить Claude»**, затем полностью перезапустите Claude.

### Claude Desktop из Microsoft Store

В разделе **«Приложения»** Claude Desktop определяется автоматически, включая пакет `AnthropicPBC.Claude` из Microsoft Store и старые установки Claude. Нажмите **«Настроить Claude»**: Helper обновит мост при необходимости и запросит права администратора для записи `REG_SZ egressProxyUrl=http://127.0.0.1:8890` в `HKLM\SOFTWARE\Policies\Claude`. Затем полностью закройте Claude через его меню в трее и откройте снова. Штатная настройка действует и при запуске обычным ярлыком Store.

По [документации Anthropic](https://claude.com/docs/third-party/claude-desktop/network-proxy), фиксированный прокси передаётся приложению, движку агента и локальной shell Cowork на Windows. Нужна версия с поддержкой `egressProxyUrl` (1.44121.1+). Это настройка маршрута, а не сетевая изоляция. Облачные/SSH-агенты используют сеть удалённой машины.

Helper сохраняет исходное значение в защищённом машинном разделе реестра. Кнопка **«Вернуть прежние настройки»** восстанавливает его, только если значение всё ещё принадлежит Helper. Чужие фиксированные и PAC-политики не перезаписываются; PAC имеет приоритет и считается конфликтом. Настройка системного прокси Windows и переменные окружения других приложений этой операцией не изменяются.

Ярлык **«Claude Desktop (proxy)»** каждый раз находит актуальную установку. Перед запуском он проверяет штатную политику и ответ `/healthz` отдельного входа: одного открытого TCP-порта недостаточно. Старые ярлыки Helper с портом 8889 переводятся на 8890. При выключенном/несовместимом мосте, конфликтующей политике или уже работающем Claude запуск покажет причину ошибки.

**Полная блокировка обхода инструментами пока не обеспечена.** Произвольная программа может игнорировать proxy-переменные. Службы, обновления, внешний браузер и сеть виртуальной машины требуют отдельного ограничения. Правила Windows Firewall по найденным путям `.exe` не покрывают автоматически новые дочерние программы, а после обновления Store пути меняются. Интерфейс отдельно показывает настройку Claude, готовность моста и отсутствие полной сетевой защиты.

Вход 8890 не использует Smart/DIRECT даже при явном `direct_domains`. Все его запросы, включая сторонние домены оплаты, идут через прокси, назначенный Claude, без переключения на другие прокси. Без назначения закрепляется первый прокси пула; противоречивые назначения Claude приводят к отказу. При ошибке или отсутствии upstream запрос завершается ошибкой. Поддерживается HTTP upstream (HTTPS-сайты через CONNECT); TLS-соединение с самим HTTPS upstream пока не реализовано, поэтому на входе Claude такая конфигурация отклоняется до отправки учётных данных. UDP/QUIC через HTTP CONNECT не передаются. Это не запрещает инструменту открыть собственное соединение вне моста.

## Программа ругается антивирусом?

Clod Helper не подписан code-signing-сертификатом (стоит $200-600/год). Поэтому SmartScreen и Defender могут проявлять осторожность. Это **не вирус** — исходники открыты в этом репозитории, можете проверить.

Если Defender пометил файл:
1. Откройте **Безопасность Windows** → **Защита от вирусов и угроз** → **Журнал защиты**.
2. Найдите заблокированный файл → **Разрешить на устройстве**.

---

## macOS

Поддерживаются Mac на Apple Silicon (M1 и новее) и на Intel, macOS 11 Big Sur и новее. В релизах лежит universal-образ `*_universal.dmg`.

**Установка.** Откройте `.dmg` и перетащите Clod Helper в «Программы». Приложение не заверено Apple (Developer ID стоит $99/год), поэтому при первом запуске macOS предупредит: нажмите на приложении ПКМ → **Открыть** → **Открыть**. На macOS 15 и новее, если кнопки «Открыть» нет: **Системные настройки → Конфиденциальность и безопасность → «Всё равно открыть»**.

Чем macOS-версия отличается от Windows:

| | Windows | macOS |
|---|---|---|
| Автозапуск моста | задача Планировщика | LaunchAgent `ru.iimperium.clodbridge` (launchd сам перезапускает упавший мост) |
| Системные прокси-переменные | `HKCU\Environment` | `launchctl setenv` (приложения из Dock/Finder) + LaunchAgent при входе + блок в `~/.zshenv` (Терминал) |
| Настройка Claude Desktop | `HKLM\SOFTWARE\Policies\Claude` (UAC) | `/Library/Managed Preferences/com.anthropic.claudefordesktop.plist` (пароль администратора) |
| Ярлыки «(proxy)» | `.lnk` | `.app` на рабочем столе или в `~/Applications` |
| Killswitch | Windows Firewall | **недоступен**: системный файрвол macOS не блокирует трафик по процессам |

Приложения, уже запущенные до включения системного проксирования, переменные не получат: их нужно перезапустить (как и на Windows). Для Терминала достаточно открыть новую вкладку.

macOS иногда удаляет файлы из `/Library/Managed Preferences`, если Mac не управляется через MDM. Если после перезагрузки настройка Claude пропала, нажмите **«Настроить Claude»** ещё раз.

---

## Для разработчиков

Стек: **Tauri 2 + React 19 + Tailwind 4 + Framer Motion**. Backend на Rust, фронт на TypeScript.

```powershell
# Установка зависимостей
npm install

# Dev-режим (hot reload фронта + Rust)
npm run tauri dev

# Релизная сборка → src-tauri/target/release/bundle/nsis/*.exe
npm run tauri build
```

На Mac (нужны Xcode Command Line Tools: `xcode-select --install`):

```bash
npm install
# Мост → src-tauri/resources/clod-bridge (universal, ad-hoc подпись)
npm run bridge:build:mac
npm run tauri dev
# Релиз → src-tauri/target/universal-apple-darwin/release/bundle/dmg/*.dmg
npm run tauri build -- --target universal-apple-darwin
```

CI (`.github/workflows/release.yml`) собирает `.exe` и `.dmg` параллельно. Отдельные macOS-настройки окна и бандла лежат в `src-tauri/tauri.macos.conf.json`, системный код macOS — в `src-tauri/src/macos.rs`.

Проверки обнаружения Claude Desktop и переноса настроек:

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo test --release --manifest-path bridge/Cargo.toml
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/desktop-launcher.Tests.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File tests/claude-proxy-policy.Tests.ps1
# Дополнительная проверка на ПК с установленным Claude Desktop (без запуска приложения):
cargo test --manifest-path src-tauri/Cargo.toml --lib detects_installed_desktop -- --ignored
```

### Уведомления всем пользователям

Рассылка — файл [`notices.json`](notices.json) в ветке `main`. Приложение (0.8.3+) скачивает его при запуске, каждые 10 минут (0.8.4+) и при открытии окна: новое всплывает в углу экрана со звуком и ложится в колокольчик в заголовке окна. Кто был офлайн, увидит при следующем запуске. GitHub кэширует файл до 5 минут — рассылка доходит за 5–15 минут. Добавьте запись и запушьте в `main` — релиз не нужен:

```json
{
  "notices": [
    {
      "id": "2026-10-23-maintenance",
      "date": "2026-10-23",
      "title": "Прокси будет недоступен 10 минут",
      "body": "Сегодня в 22:00 по Москве — плановые работы у провайдера.",
      "link": "https://t.me/your_channel",
      "platforms": ["windows"],
      "min_version": "0.8.3"
    }
  ]
}
```

`id` — уникальный и неизменный (по нему помнится «прочитано»). Необязательные поля: `link` (только `https://`), `platforms` (`windows` / `macos`), `min_version` / `max_version`. Удалённая из файла запись пропадает из колокольчика.

### Структура проекта

```
src/                    React frontend
  wizard/               5 экранов мастера установки
  components/           общие UI-компоненты (Button, SuccessCheck, StatusPill)
  lib/                  типы + API-обёртки Tauri-команд
src-tauri/              Rust backend
  src/commands.rs       Tauri-команды (#[tauri::command])
  src/proxy.rs          парсинг + проверка прокси через reqwest + ip-api.com
  src/system.rs         детект Node.js / Python
  resources/            bundled ресурсы:
    claude-setup.ps1            установщик Claude Code
    claude-desktop-setup.ps1    установщик локального моста (+ ярлык Claude Desktop)
    claude-desktop-launch.ps1   поиск Store/legacy Claude и запуск через прокси
    cursor-setup.ps1            ярлык «Cursor (proxy)» (мост общий)
    clod-bridge.exe             мост Rust с двумя входами и авторизацией upstream
    claude-proxy-policy.ps1     обратимая штатная политика Claude в HKLM
  tauri.conf.json       bundle metadata (publisher / copyright / иконки)
```

### Архитектура моста

```
Claude Desktop + агент + локальный Cowork
    → 127.0.0.1:8890 → clod-bridge.exe → HTTP upstream → сайт
                       только upstream, без DIRECT

Другие настроенные приложения
    → 127.0.0.1:8889 → clod-bridge.exe → правила Smart/Full

Инструменты, игнорирующие proxy-настройки, этим мостом не ограничиваются.
```

Почему мост нужен: Electron-приложения плохо переваривают `http://user:pass@host:port` в `--proxy-server`. Мост принимает запросы **без auth** на localhost и инжектит `Proxy-Authorization: Basic <base64>` перед форвардом в upstream.

### Лицензия

MIT — см. [LICENSE](LICENSE).
