//! Фото и видео баннеров из админки.
//!
//! Файл лежит на сервере статистики под ключом «sha256.расширение». Программа
//! скачивает его сама: всегда сначала напрямую, чтобы не тратить трафик
//! прокси человека. Картинка зависла или оборвалась - ещё раз через мост;
//! видео через мост не качается вовсе (см. `telemetry::media_routes`). Дальше
//! программа сверяет сумму с
//! ключом и кладёт в кэш на диске. Интерфейс показывает файл из кэша: чужие
//! сайты IP человека не видят, подменить файл нельзя, видео открывается и без
//! сети после первой загрузки. Файлы, которых нет в баннерах, удаляются.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Больше этого не качаем (на сервере предел 24 МБ).
const MAX_BYTES: usize = 25 * 1024 * 1024;
/// Видео до 24 МБ на медленном канале (на каждый путь; зависание ловится
/// раньше, по паузе без новых байт).
const TIMEOUT: Duration = Duration::from_secs(180);

const EXTS: [&str; 6] = ["jpg", "png", "webp", "gif", "mp4", "webm"];

/// Ключ наш: 64 hex-символа, точка, известное расширение.
pub fn valid_key(key: &str) -> bool {
    let Some((hash, ext)) = key.split_once('.') else { return false };
    hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) && EXTS.contains(&ext)
}

/// Вид файла по ключу.
pub fn kind_of(key: &str) -> Option<&'static str> {
    match key.rsplit_once('.')?.1 {
        "mp4" | "webm" => Some("video"),
        "jpg" | "png" | "webp" | "gif" => Some("image"),
        _ => None,
    }
}

fn dir() -> Result<PathBuf, String> {
    Ok(dirs::cache_dir().ok_or("Нет папки кэша")?.join("ru.iimperium.clodhelper").join("media"))
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect()
}

/// Совпадает ли содержимое с ключом.
pub fn matches_key(key: &str, data: &[u8]) -> bool {
    key.split_once('.').is_some_and(|(hash, _)| sha256_hex(data) == hash)
}

/// Путь к файлу в кэше; нет - скачать и проверить.
pub async fn ensure(key: &str) -> Result<PathBuf, String> {
    if !valid_key(key) {
        return Err("неизвестный файл".into());
    }
    let path = dir()?.join(key);
    if path.is_file() {
        return Ok(path);
    }
    let setup = crate::notices::is_setup_media(key);
    let data = crate::telemetry::get_bytes(&format!("/v1/media/{key}"), MAX_BYTES, TIMEOUT, setup, kind_of(key) == Some("video")).await?;
    if !matches_key(key, &data) {
        return Err("файл повреждён".into());
    }
    let key = key.to_string();
    tokio::task::spawn_blocking(move || -> Result<PathBuf, String> {
        let dir = dir()?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        // Сначала во временный файл: оборванная запись не выдаст себя за готовую.
        let tmp = dir.join(format!("{key}.part"));
        std::fs::write(&tmp, &data).map_err(|e| e.to_string())?;
        let path = dir.join(&key);
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Несколько файлов разом: ключ - путь (не скачалось - нет в ответе).
pub async fn ensure_many(keys: Vec<String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for k in keys.into_iter().take(16) {
        match ensure(&k).await {
            Ok(p) => {
                out.insert(k, p.to_string_lossy().into_owned());
            }
            Err(e) => tracing::info!("медиа баннера {k}: {e}"),
        }
    }
    out
}

/// Удалить из кэша всё, чего нет в `keep` (и недокачанное).
pub fn prune(keep: &[String]) {
    let Ok(dir) = dir() else { return };
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !keep.contains(&name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_strict() {
        let h = "a".repeat(64);
        assert!(valid_key(&format!("{h}.jpg")));
        assert!(valid_key(&format!("{h}.mp4")));
        assert!(!valid_key(&format!("{h}.exe")));
        assert!(!valid_key(&format!("{}.jpg", "A".repeat(64))), "только строчные hex");
        assert!(!valid_key(&format!("{}.jpg", "a".repeat(63))));
        assert!(!valid_key("../../etc/passwd"));
        assert!(!valid_key(&format!("{h}.jpg/../x")));
        assert_eq!(kind_of(&format!("{h}.webm")), Some("video"));
        assert_eq!(kind_of(&format!("{h}.png")), Some("image"));
    }

    #[test]
    fn content_must_match_key() {
        let data = b"hello";
        let key = format!("{}.png", sha256_hex(data));
        assert!(matches_key(&key, data));
        assert!(!matches_key(&key, b"hellO"));
    }
}
