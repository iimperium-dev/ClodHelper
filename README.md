# Clod Helper

Утилита для установки **Claude Code** и **Claude Desktop** через прокси.
Нужна тем, у кого Anthropic API блокирует напрямую (например, в России).

Работает через локальный мост `127.0.0.1:8889`, который форвардит трафик в купленный HTTP-прокси с автоматической авторизацией. Электрон-приложения (включая Claude Desktop) ходят через мост — система не трогается, остальной интернет работает напрямую.

---

## Скачать

[Последний релиз →](https://github.com/mogylnyy/Clod-Helper-v1/releases)

Файл `Clod_Helper_*_x64-setup.exe` — ~10 МБ, Windows 10/11.

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
- **Python 3.7+** — нужен мосту прокси. Тоже подскажет.

После установки Claude Code запускается командой `claude` в любом терминале. Claude Desktop — через ярлык **«Claude Desktop (proxy)»** на рабочем столе (обычный ярлык Claude не пройдёт через прокси).

## Программа ругается антивирусом?

Clod Helper не подписан code-signing-сертификатом (стоит $200-600/год). Поэтому SmartScreen и Defender могут проявлять осторожность. Это **не вирус** — исходники открыты в этом репозитории, можете проверить.

Если Defender пометил файл:
1. Откройте **Безопасность Windows** → **Защита от вирусов и угроз** → **Журнал защиты**.
2. Найдите заблокированный файл → **Разрешить на устройстве**.

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
    claude-desktop-setup.ps1    установщик локального моста
    local-proxy.py              сам мост (Python HTTP-proxy с auth-injection)
  tauri.conf.json       bundle metadata (publisher / copyright / иконки)
```

### Архитектура моста

```
┌────────────────┐         ┌─────────────────────────┐         ┌──────────────────┐
│ Claude Desktop │  HTTP   │ local-proxy.py          │  HTTP+  │ купленный прокси │
│ Claude Code    │ ──────▶ │ 127.0.0.1:8889          │  auth   │ login:pass@      │
│ VS Code        │         │ без auth, ловит CONNECT │ ──────▶ │ x.x.x.x:port     │
└────────────────┘         └─────────────────────────┘         └────────┬─────────┘
                                                                        │
                                                                        ▼
                                                              ┌─────────────────┐
                                                              │ api.anthropic.com │
                                                              └─────────────────┘
```

Почему мост нужен: Electron-приложения плохо переваривают `http://user:pass@host:port` в `--proxy-server`. Мост принимает запросы **без auth** на localhost и инжектит `Proxy-Authorization: Basic <base64>` перед форвардом в upstream.

### Лицензия

MIT — см. [LICENSE](LICENSE).
