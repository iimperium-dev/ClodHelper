//! Чистка НАШИХ ключей из settings.json VS Code (легаси Фазы 0).
//!
//! Раньше хелпер прописывал `http.proxy` + `terminal.integrated.env.windows`,
//! потому что прокси жил только в env дочерних процессов лаунчера, а VS Code
//! пересобирает окружение терминала и env не наследовал. Теперь прокси
//! доставляется глобальными User env-переменными (env_proxy.rs) — терминал
//! VS Code наследует их сам, ключи в settings.json стали лишними дублями.
//!
//! Остался только `disable()`: убрать наши ключи у существующих установок
//! (вызывается best-effort при включении глобального прокси). Чужие настройки
//! не трогаем; JSONC (комментарии) не парсим — тогда просто выходим.

use std::path::PathBuf;

use serde_json::{Map, Value};

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

/// Прочитать settings.json как JSON-объект (пустой, если файла нет/битый).
fn read_settings(path: &std::path::Path) -> Map<String, Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Map::new();
    };
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

/// Убрать наши ключи из settings.json VS Code, чужие не трогать.
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
