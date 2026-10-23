//! Обновление Clod Helper в один клик (без подписи Apple и Microsoft).
//!
//! Проверка — по манифесту `latest.json` последнего релиза GitHub
//! (iimperium-dev/ClodHelper): ссылка `releases/latest/download/…` идёт
//! через сайт и CDN, а не через API — без лимита «60 запросов в час на IP».
//! Манифест подписан ключом разработчика (ed25519, `latest.json.sig`;
//! подписывает scripts/publish-release.mjs на его машине — на GitHub
//! закрытого ключа нет). Подпись не сошлась — обновление не ставим: так не
//! пройдёт подмена файлов релиза (утёкший токен, взлом аккаунта, зеркало).
//! В манифесте — SHA-256 установщиков; скачанный файл сверяется с ним:
//!   - macOS: `*_universal.dmg` — новая версия встаёт на место текущей,
//!     старая уходит в ~/Library/Application Support/ClodHelper-backup;
//!   - Windows: `*_x64-setup.exe` (NSIS) — тихая установка поверх текущей
//!     в профиль пользователя (без UAC), новая версия стартует сама.

use serde::{Deserialize, Serialize};

const REPO: &str = "iimperium-dev/ClodHelper";

/// Открытые ключи подписи манифеста (ed25519, base64 сырых 32 байт).
/// Несколько — на время смены ключа: новый добавить сюда заранее, выпустить
/// версию, и только потом подписывать новым.
const UPDATE_KEYS: &[&str] = &["dkN/Vz8GRDntm9Ra94VI+slYMi3kgsIv2rbZoB8hjow="];

const MANIFEST: &str = "latest.json";

/// Платформа этой сборки в манифесте.
#[cfg(not(target_os = "windows"))]
const PLATFORM: &str = "darwin-universal";
#[cfg(target_os = "windows")]
const PLATFORM: &str = "windows-x86_64";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    /// Установщик для этой ОС (.dmg или -setup.exe).
    pub url: String,
    /// SHA-256 установщика из подписанного манифеста.
    #[serde(default)]
    pub sha256: String,
    /// Пункты «Что нового» этой версии.
    #[serde(default)]
    pub highlights: Vec<String>,
}
/// Ход установки для UI: событие `update:progress`.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgress {
    /// download → verify → install → restart.
    pub stage: &'static str,
    pub received: u64,
    /// Размер установщика; 0 — сервер не сообщил.
    pub total: u64,
}

/// Манифест обновления (latest.json; собирает CI, подписывает разработчик).
#[derive(Debug, Deserialize)]
struct Manifest {
    version: String,
    tag: String,
    #[serde(default)]
    highlights: Vec<String>,
    platforms: std::collections::HashMap<String, ManifestFile>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    /// Имя файла в релизе (как его назвал GitHub — пробелы стали точками).
    file: String,
    sha256: String,
    /// «Что нового» именно для этой ОС (manifest ≥0.8.11). Пустой список —
    /// в релизе для этой ОС ничего нового: обновление не предлагаем, люди не
    /// получают уведомление о версии, которая им ничего не даёт. Нет поля —
    /// старый манифест: берём общий список.
    #[serde(default)]
    highlights: Option<Vec<String>>,
}

/// "0.6.0" / "v0.6.1" → (0, 6, 1).
pub fn parse_ver(v: &str) -> Option<(u32, u32, u32)> {
    let mut it = v.trim().trim_start_matches('v').split('.').map(|p| p.parse::<u32>().ok());
    Some((it.next()??, it.next()??, it.next().flatten().unwrap_or(0)))
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .user_agent(concat!("ClodHelper/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| format!("Обновления: {e}"))
}

/// Есть ли версия новее текущей. None — уже последняя.
pub async fn check() -> Result<Option<UpdateInfo>, String> {
    let m = fetch_manifest().await.map_err(|e| format!("Не удалось проверить обновления: {e}"))?;
    Ok(update_from(m, env!("CARGO_PKG_VERSION"), PLATFORM))
}

/// Манифест последнего релиза — только с верной подписью.
async fn fetch_manifest() -> Result<Manifest, String> {
    let c = client()?;
    let base = format!("https://github.com/{REPO}/releases/latest/download");
    let get = |url: String| {
        let c = c.clone();
        async move {
            let r = c.get(&url).send().await.map_err(|e| format!("нет связи с GitHub ({e})"))?;
            if r.status() == reqwest::StatusCode::NOT_FOUND {
                return Err("в последнем релизе нет подписанного манифеста".to_string());
            }
            let r = r.error_for_status().map_err(|e| format!("GitHub: {e}"))?;
            r.bytes().await.map_err(|e| format!("GitHub: {e}"))
        }
    };
    let body = get(format!("{base}/{MANIFEST}")).await?;
    let sig = get(format!("{base}/{MANIFEST}.sig")).await?;
    verify_manifest(&body, &sig, UPDATE_KEYS)?;
    serde_json::from_slice(&body).map_err(|e| format!("манифест не читается: {e}"))
}

/// Подпись манифеста (base64 64 байт ed25519) — одним из наших ключей.
fn verify_manifest(body: &[u8], sig: &[u8], keys: &[&str]) -> Result<(), String> {
    use base64::Engine;
    use ed25519_dalek::{Signature, VerifyingKey};
    let b64 = base64::engine::general_purpose::STANDARD;
    let bad = || "Подпись обновления не сошлась — не ставлю (файлы релиза могли подменить)".to_string();
    let sig = b64.decode(String::from_utf8_lossy(sig).trim()).map_err(|_| bad())?;
    let sig = Signature::from_slice(&sig).map_err(|_| bad())?;
    let ok = keys.iter().any(|k| {
        let Ok(raw) = b64.decode(k) else { return false };
        let Ok(raw): Result<[u8; 32], _> = raw.try_into() else { return false };
        VerifyingKey::from_bytes(&raw).is_ok_and(|vk| vk.verify_strict(body, &sig).is_ok())
    });
    if ok { Ok(()) } else { Err(bad()) }
}

/// Обновление из проверенного манифеста: новее `current`, со своим
/// установщиком. Ссылка — на файл того же релиза.
fn update_from(m: Manifest, current: &str, platform: &str) -> Option<UpdateInfo> {
    if parse_ver(&m.version)? <= parse_ver(current)? {
        return None;
    }
    // Тег и версия подписаны вместе — должны совпадать.
    if m.tag.trim_start_matches('v') != m.version {
        return None;
    }
    let f = m.platforms.get(platform)?;
    if f.highlights.as_ref().is_some_and(|h| h.is_empty()) {
        return None;
    }
    let sha = f.sha256.to_lowercase();
    let safe_name = !f.file.is_empty() && !f.file.contains(['/', '\\']) && !f.file.contains("..");
    if !safe_name || sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(UpdateInfo {
        url: format!("https://github.com/{REPO}/releases/download/{}/{}", m.tag, f.file),
        version: m.version,
        notes: String::new(),
        sha256: sha,
        highlights: f.highlights.clone().unwrap_or(m.highlights).into_iter().take(12).collect(),
    })
}
/// Скачать установщик и проверить его SHA-256. Возвращает имя файла и байты.
/// Качаем кусками и сообщаем ход в `progress`: без этого окно минутами
/// крутило спиннер, и обновление выглядело зависшим.
async fn download_verified(
    info: &UpdateInfo,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
) -> Result<(String, Vec<u8>), String> {
    use sha2::{Digest, Sha256};
    // Адресу и сумме из окна не доверяем: заново берём подписанный манифест
    // и ставим ровно то, что в нём (и ту же версию, что видел пользователь).
    let trusted = check()
        .await?
        .filter(|t| t.version == info.version)
        .ok_or("Обновление на сервере изменилось — проверьте ещё раз")?;
    let c = client()?;
    let file = urlencoding_decode(trusted.url.rsplit('/').next().unwrap_or_default());
    let want = trusted.sha256.clone();
    let mut resp = c
        .get(&trusted.url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Загрузка: {e}"))?;
    let total = resp.content_length().unwrap_or(0);
    let mut bytes = Vec::with_capacity(total as usize);
    let mut hasher = Sha256::new();
    progress(UpdateProgress { stage: "download", received: 0, total });
    let mut last = std::time::Instant::now();
    while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Загрузка: {e}"))? {
        hasher.update(&chunk);
        bytes.extend_from_slice(&chunk);
        // Не чаще ~12 раз в секунду: событие на каждый кусок в 16 КБ
        // забивало бы IPC без пользы для глаза.
        if last.elapsed() >= std::time::Duration::from_millis(80) {
            last = std::time::Instant::now();
            progress(UpdateProgress { stage: "download", received: bytes.len() as u64, total });
        }
    }
    let got = bytes.len() as u64;
    progress(UpdateProgress { stage: "verify", received: got, total: total.max(got) });
    if format!("{:x}", hasher.finalize()) != want {
        return Err("Файл обновления повреждён (контрольная сумма не совпала) — не ставлю".into());
    }
    Ok((file, bytes))
}

/// Чистая папка для скачанного обновления.
fn work_dir() -> Result<std::path::PathBuf, String> {
    let work = dirs::cache_dir()
        .ok_or("Нет папки кэша")?
        .join("ru.iimperium.clodhelper")
        .join("update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    Ok(work)
}

/// Скачать, проверить, поставить и перезапуститься (после вызова приложение
/// должно завершиться — новую версию запустит фоновый скрипт).
#[cfg(not(target_os = "windows"))]
pub async fn install(
    info: &UpdateInfo,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
) -> Result<(), String> {
    use std::path::PathBuf;

    // Где стоит текущее приложение: …/Clod Helper.app/Contents/MacOS/clod-helper
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle: PathBuf = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(PathBuf::from)
        .ok_or("Обновление работает только для установленного приложения (.app)")?;

    let (_, bytes) = download_verified(info, progress).await?;
    let size = bytes.len() as u64;
    progress(UpdateProgress { stage: "install", received: size, total: size });
    let work = work_dir()?;
    let dmg = work.join("update.dmg");
    std::fs::write(&dmg, &bytes).map_err(|e| e.to_string())?;

    // Смонтировать, скопировать новую версию рядом, отмонтировать.
    let mount = work.join("mnt");
    let ok = std::process::Command::new("/usr/bin/hdiutil")
        .args(["attach", "-nobrowse", "-readonly", "-mountpoint"])
        .arg(&mount)
        .arg(&dmg)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        return Err("Не удалось открыть образ обновления".into());
    }
    let staged = bundle.with_extension("app.new");
    let _ = std::fs::remove_dir_all(&staged);
    let copied = std::process::Command::new("/usr/bin/ditto")
        .arg(mount.join("Clod Helper.app"))
        .arg(&staged)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    let _ = std::process::Command::new("/usr/bin/hdiutil").args(["detach", "-quiet"]).arg(&mount).status();
    if !copied {
        return Err("Не удалось распаковать обновление".into());
    }

    // Замена после выхода: старую — в резервные копии, новую — на место.
    let backup_dir = dirs::data_dir()
        .ok_or("Нет папки данных")?
        .join("ClodHelper-backup");
    let q = |p: &std::path::Path| crate::macos::sh_quote(&p.to_string_lossy());
    let script = format!(
        "pid={pid}\nwhile kill -0 $pid 2>/dev/null; do sleep 0.3; done\n\
         mkdir -p {bdir}\nrm -rf {bold}\nmv {app} {bold} && mv {new} {app} || mv {bold} {app}\n\
         xattr -dr com.apple.quarantine {app} 2>/dev/null\nopen {app}\n",
        pid = std::process::id(),
        bdir = q(&backup_dir),
        bold = q(&backup_dir.join(format!("Clod Helper ({}).app", env!("CARGO_PKG_VERSION")))),
        app = q(&bundle),
        new = q(&staged),
    );
    let sh = work.join("swap.sh");
    std::fs::write(&sh, script).map_err(|e| e.to_string())?;
    std::process::Command::new("/usr/bin/nohup")
        .arg("/bin/sh")
        .arg(&sh)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Не удалось запустить установку: {e}"))?;
    Ok(())
}

/// Ключи NSIS-установщика Tauri (как у его штатного апдейтера): /S — без
/// окон (белое окно /P выглядело чужеродно — вместо него наша заставка,
/// update_splash.rs); /UPDATE — поверх текущей версии, без её удаления и
/// без пересоздания ярлыков, которые пользователь мог убрать; /R —
/// запустить новую версию после установки.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const NSIS_UPDATE_ARGS: [&str; 3] = ["/S", "/UPDATE", "/R"];

/// Windows: скачать и проверить установщик, запустить его. Установка
/// в профиль пользователя (currentUser) — без UAC. Приложение сразу после
/// этого выходит (update_install); установщик дождётся выхода — запущенную
/// копию он и сам закрывает — и запустит новую версию. Мост работает из
/// своей папки и обновится при первом старте новой версии.
#[cfg(target_os = "windows")]
pub async fn install(
    info: &UpdateInfo,
    progress: &(dyn Fn(UpdateProgress) + Send + Sync),
) -> Result<(), String> {
    let (_, bytes) = download_verified(info, progress).await?;
    let size = bytes.len() as u64;
    progress(UpdateProgress { stage: "install", received: size, total: size });
    let work = work_dir()?;
    let setup = work.join("clod-helper-setup.exe");
    std::fs::write(&setup, &bytes).map_err(|e| format!("Не удалось сохранить установщик: {e}"))?;
    let child = std::process::Command::new(&setup)
        .args(NSIS_UPDATE_ARGS)
        .spawn()
        .map_err(|e| format!("Не удалось запустить установщик: {e}"))?;
    // Заставка — украшение: не запустилась, обновление всё равно идёт.
    if let Err(e) = crate::update_splash::spawn(&work, child.id(), &info.version) {
        tracing::warn!("{e}");
    }
    Ok(())
}

/// %20 → пробел и т.п. (имена ассетов GitHub в URL экранированы).
fn urlencoding_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={s}").as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    fn keypair(seed: u8) -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[seed; 32]);
        let pk = base64::engine::general_purpose::STANDARD.encode(sk.verifying_key().to_bytes());
        (sk, pk)
    }

    fn signed(sk: &SigningKey, body: &[u8]) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD.encode(sk.sign(body).to_bytes()).into_bytes()
    }

    #[test]
    fn manifest_signature_is_required_and_checked() {
        let (sk, pk) = keypair(7);
        let (other, other_pk) = keypair(9);
        let body = br#"{"version":"0.9.0"}"#;
        assert!(verify_manifest(body, &signed(&sk, body), &[&pk]).is_ok());
        // Ключ на смену: годится любой из списка.
        assert!(verify_manifest(body, &signed(&sk, body), &[&other_pk, &pk]).is_ok());
        // Чужой ключ, изменённый манифест, мусор вместо подписи — отказ.
        assert!(verify_manifest(body, &signed(&other, body), &[&pk]).is_err());
        assert!(verify_manifest(br#"{"version":"9.9.9"}"#, &signed(&sk, body), &[&pk]).is_err());
        assert!(verify_manifest(body, b"not base64!", &[&pk]).is_err());
        assert!(verify_manifest(body, b"", &[&pk]).is_err());
    }

    #[test]
    fn real_update_key_is_valid() {
        for k in UPDATE_KEYS {
            let raw = base64::engine::general_purpose::STANDARD.decode(k).unwrap();
            let raw: [u8; 32] = raw.try_into().unwrap();
            assert!(ed25519_dalek::VerifyingKey::from_bytes(&raw).is_ok());
        }
    }

    fn manifest(version: &str, tag: &str, file: &str, sha: &str) -> Manifest {
        serde_json::from_value(serde_json::json!({
            "version": version, "tag": tag, "highlights": ["Новое"],
            "platforms": {
                "windows-x86_64": { "file": file, "sha256": sha },
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64) },
            }
        }))
        .unwrap()
    }

    #[test]
    fn update_comes_only_from_a_consistent_newer_manifest() {
        let sha = "A".repeat(64);
        let exe = "Clod.Helper_0.9.0_x64-setup.exe";
        let win = update_from(manifest("0.9.0", "v0.9.0", exe, &sha), "0.8.8", "windows-x86_64").unwrap();
        assert_eq!(win.url, format!("https://github.com/{REPO}/releases/download/v0.9.0/{exe}"));
        assert_eq!(win.sha256, "a".repeat(64));
        assert_eq!(win.highlights, ["Новое"]);
        let mac = update_from(manifest("0.9.0", "v0.9.0", exe, &sha), "0.8.8", "darwin-universal").unwrap();
        assert!(mac.url.ends_with("_universal.dmg"));
        // Не новее; тег не тот; путь в имени файла; не сумма; нет своей ОС.
        assert!(update_from(manifest("0.8.8", "v0.8.8", exe, &sha), "0.8.8", "windows-x86_64").is_none());
        assert!(update_from(manifest("0.9.0", "v0.9.1", exe, &sha), "0.8.8", "windows-x86_64").is_none());
        assert!(update_from(manifest("0.9.0", "v0.9.0", "../evil.exe", &sha), "0.8.8", "windows-x86_64").is_none());
        assert!(update_from(manifest("0.9.0", "v0.9.0", exe, "abc"), "0.8.8", "windows-x86_64").is_none());
        assert!(update_from(manifest("0.9.0", "v0.9.0", exe, &sha), "0.8.8", "linux").is_none());

        // Свои пункты у ОС: пусто — для этой ОС ничего нового, не предлагаем.
        let per_os: Manifest = serde_json::from_value(serde_json::json!({
            "version": "0.9.0", "tag": "v0.9.0", "highlights": ["Только Windows"],
            "platforms": {
                "windows-x86_64": { "file": exe, "sha256": sha, "highlights": ["Только Windows"] },
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64), "highlights": [] },
            }
        }))
        .unwrap();
        assert!(update_from(per_os, "0.8.8", "darwin-universal").is_none(), "Mac не зовём на версию только для Windows");
        let per_os: Manifest = serde_json::from_value(serde_json::json!({
            "version": "0.9.0", "tag": "v0.9.0", "highlights": ["Общее", "Для Mac"],
            "platforms": {
                "darwin-universal": { "file": "Clod.Helper_0.9.0_universal.dmg", "sha256": "b".repeat(64), "highlights": ["Общее", "Для Mac"] },
                "windows-x86_64": { "file": exe, "sha256": sha, "highlights": ["Общее"] },
            }
        }))
        .unwrap();
        assert_eq!(update_from(per_os, "0.8.8", "windows-x86_64").unwrap().highlights, ["Общее"]);
        assert!(parse_ver("v0.7.0") > parse_ver("0.6.0"));
        assert!(parse_ver("0.6.10") > parse_ver("0.6.9"));
        assert_eq!(urlencoding_decode("Clod%20Helper_0.6.0.dmg"), "Clod Helper_0.6.0.dmg");
    }

    #[test]
    fn windows_update_is_silent_in_place_and_restarts() {
        assert_eq!(NSIS_UPDATE_ARGS, ["/S", "/UPDATE", "/R"]);
    }

    /// Живая проверка: cargo test --lib live_update_check -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_update_check() {
        println!("update → {:?}", check().await);
    }
}