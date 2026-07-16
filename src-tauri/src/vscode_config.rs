//! Правка settings.json VS Code — прокси для интегрированного терминала.
//!
//! Зачем отдельно от launcher: для VS Code флага `--proxy-server` НЕ хватает.
//! Он проксирует только Chromium-часть VS Code (UI, встроенный fetch), но
//! интегрированный ТЕРМИНАЛ — где живёт Claude Code и всё, что он запускает
//! (bash/node/git/python) — env от процесса Code.exe НЕ наследует надёжно:
//! VS Code резолвит окружение терминала заново. Проверено вживую: claude.exe
//! в дереве VS Code имел пустой HTTP_PROXY несмотря на cmd.env при запуске.
//!
//! Решение: прописать в settings.json VS Code
//!   - `http.proxy` — для самого VS Code (расширения, обновления),
//!   - `terminal.integrated.env.windows` — HTTP(S)_PROXY для КАЖДОГО нового
//!     терминала → Claude Code внутри получает прокси автоматически.
//! Применяется к терминалам, открытым ПОСЛЕ сохранения (существующие — нет).
//!
//! JSON правим сохраняя остальные настройки пользователя. Бэкап перед первой
//! правкой. Ключи наши (http.proxy, terminal.integrated.env.windows) при
//! выключении убираем, чужие не трогаем.

use std::path::PathBuf;

use serde_json::{Map, Value};

use crate::shim::LISTEN_PORT;

/// Путь к пользовательскому settings.json VS Code.
fn settings_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(appdata)
            .join("Code")
            .join("User")
            .join("settings.json"),
    )
}

fn proxy_url() -> String {
    format!("http://127.0.0.1:{LISTEN_PORT}")
}

/// Прочитать settings.json как JSON-объект (пустой, если файла нет/битый).
fn read_settings(path: &std::path::Path) -> Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Map::new();
    };
    // VS Code допускает комментарии/trailing commas (JSONC). serde_json их не
    // ест. Пробуем строгий парс; если не вышло — не трогаем файл (вернём флаг
    // через ошибку выше по стеку), чтобы не сломать пользовательский JSONC.
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn write_settings(path: &std::path::Path, map: &Map<String, Value>) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Не создать папку {}: {e}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(map.clone()))
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, text).map_err(|e| format!("Не записать settings: {e}"))
}

/// Включить прокси для VS Code: прописать http.proxy + terminal env.
/// Возвращает true если файл был изменён.
pub fn enable() -> Result<bool, String> {
    let path = settings_path().ok_or_else(|| "APPDATA не задан".to_string())?;

    // Файл существует и НЕ парсится строгим JSON (значит JSONC с комментариями)
    // — не рискуем перезаписью, сообщаем честно.
    if path.exists() {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if !text.trim().is_empty() && serde_json::from_str::<Value>(&text).is_err() {
            return Err(
                "settings.json VS Code содержит комментарии/JSONC — не могу править \
                 автоматически. Пропишите вручную terminal.integrated.env.windows."
                    .into(),
            );
        }
    }

    // Бэкап перед первой правкой (если ещё нет).
    if path.exists() {
        let bak = path.with_extension("json.clodbak");
        if !bak.exists() {
            let _ = std::fs::copy(&path, &bak);
        }
    }

    let mut map = read_settings(&path);
    let proxy = proxy_url();

    map.insert("http.proxy".into(), Value::String(proxy.clone()));

    let mut term_env = Map::new();
    term_env.insert("HTTP_PROXY".into(), Value::String(proxy.clone()));
    term_env.insert("HTTPS_PROXY".into(), Value::String(proxy.clone()));
    term_env.insert(
        "NO_PROXY".into(),
        Value::String("localhost,127.0.0.1,::1".into()),
    );
    map.insert(
        "terminal.integrated.env.windows".into(),
        Value::Object(term_env),
    );

    write_settings(&path, &map)?;
    Ok(true)
}

/// Выключить прокси для VS Code: убрать наши ключи, чужие не трогать.
pub fn disable() -> Result<bool, String> {
    let path = settings_path().ok_or_else(|| "APPDATA не задан".to_string())?;
    if !path.exists() {
        return Ok(false);
    }
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.trim().is_empty() && serde_json::from_str::<Value>(&text).is_err() {
        // JSONC — не трогаем.
        return Ok(false);
    }
    let mut map = read_settings(&path);
    let had = map.remove("http.proxy").is_some()
        | map.remove("terminal.integrated.env.windows").is_some();
    if had {
        write_settings(&path, &map)?;
    }
    Ok(had)
}

/// Прописан ли уже наш прокси в settings.json VS Code.
/// Задел для UI-индикатора «терминал VS Code проксируется».
#[allow(dead_code)]
pub fn is_enabled() -> bool {
    let Some(path) = settings_path() else {
        return false;
    };
    let map = read_settings(&path);
    map.get("http.proxy")
        .and_then(|v| v.as_str())
        .map(|s| s.contains(&format!(":{LISTEN_PORT}")))
        .unwrap_or(false)
}
