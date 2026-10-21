//! Обновление Clod Helper в один клик (без подписи Apple и Microsoft).
//!
//! Проверка: последний релиз GitHub (iimperium-dev/Clod-Helper-v1). Если он
//! новее — по кнопке скачиваем установщик своей ОС и сверяем SHA-256 с
//! `SHA256SUMS.txt` того же релиза (CI публикует его рядом):
//!   - macOS: `*_universal.dmg` — новая версия встаёт на место текущей,
//!     старая уходит в ~/Library/Application Support/ClodHelper-backup;
//!   - Windows: `*_x64-setup.exe` (NSIS) — тихая установка поверх текущей
//!     в профиль пользователя (без UAC), новая версия стартует сама.

use serde::{Deserialize, Serialize};

const REPO: &str = "iimperium-dev/Clod-Helper-v1";

/// Установщик этой ОС среди файлов релиза.
#[cfg(not(target_os = "windows"))]
const INSTALLER_SUFFIX: &str = "_universal.dmg";
#[cfg(target_os = "windows")]
const INSTALLER_SUFFIX: &str = "_x64-setup.exe";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    /// Установщик для этой ОС (.dmg или -setup.exe).
    pub url: String,
    pub sums_url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
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
    let r: Release = client()?
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .send()
        .await
        .map_err(|e| format!("Не удалось проверить обновления: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Не удалось проверить обновления: {e}"))?
        .json()
        .await
        .map_err(|e| format!("Обновления: {e}"))?;
    Ok(pick_update(r, env!("CARGO_PKG_VERSION"), INSTALLER_SUFFIX))
}

/// Обновление из релиза: только опубликованный, новее `current` и с
/// установщиком `suffix` и контрольными суммами рядом.
fn pick_update(r: Release, current: &str, suffix: &str) -> Option<UpdateInfo> {
    if r.draft || r.prerelease {
        return None;
    }
    if parse_ver(&r.tag_name)? <= parse_ver(current)? {
        return None;
    }
    let find = |pred: &dyn Fn(&str) -> bool| r.assets.iter().find(|a| pred(&a.name)).map(|a| a.browser_download_url.clone());
    // Без контрольной суммы не ставим — нечем проверить целостность.
    let url = find(&|n| n.ends_with(suffix))?;
    let sums_url = find(&|n| n == "SHA256SUMS.txt")?;
    Some(UpdateInfo {
        version: r.tag_name.trim_start_matches('v').to_string(),
        notes: r.body.unwrap_or_default().chars().take(600).collect(),
        url,
        sums_url,
    })
}

/// Сумма файла из SHA256SUMS: строки «<hex>  <имя>» (sha256sum; «*» перед
/// именем — бинарный режим). Имя — весь остаток строки: в нём бывают
/// пробелы. GitHub заменяет пробелы в именах файлов релиза точками («Clod
/// Helper_0.7.0_x64-setup.exe» → «Clod.Helper_…»), а суммы считаются до
/// загрузки — поэтому имена сравниваются без этой разницы.
pub fn expected_sum(sums: &str, file: &str) -> Option<String> {
    let norm = |s: &str| s.trim().replace(' ', ".");
    sums.lines().find_map(|l| {
        let (sum, name) = l.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (sum.len() == 64 && sum.chars().all(|c| c.is_ascii_hexdigit()) && norm(name) == norm(file))
            .then(|| sum.to_lowercase())
    })
}

/// Скачать установщик и проверить его SHA-256. Возвращает имя файла и байты.
async fn download_verified(info: &UpdateInfo) -> Result<(String, Vec<u8>), String> {
    use sha2::{Digest, Sha256};
    let c = client()?;
    let sums = c
        .get(&info.sums_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Контрольные суммы: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Контрольные суммы: {e}"))?;
    let file = urlencoding_decode(info.url.rsplit('/').next().unwrap_or_default());
    let want = expected_sum(&sums, &file).ok_or("В релизе нет контрольной суммы установщика")?;
    let bytes = c
        .get(&info.url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Загрузка: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Загрузка: {e}"))?;
    if format!("{:x}", Sha256::digest(&bytes)) != want {
        return Err("Файл обновления повреждён (контрольная сумма не совпала) — не ставлю".into());
    }
    Ok((file, bytes.to_vec()))
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
pub async fn install(info: &UpdateInfo) -> Result<(), String> {
    use std::path::PathBuf;

    // Где стоит текущее приложение: …/Clod Helper.app/Contents/MacOS/clod-helper
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle: PathBuf = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(PathBuf::from)
        .ok_or("Обновление работает только для установленного приложения (.app)")?;

    let (_, bytes) = download_verified(info).await?;
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

/// Ключи тихой установки NSIS-установщика Tauri (как у его штатного
/// апдейтера): /S — без окон; /UPDATE — поверх текущей версии, без её
/// удаления и без пересоздания ярлыков, которые пользователь мог убрать;
/// /R — запустить новую версию после установки.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
const NSIS_UPDATE_ARGS: [&str; 3] = ["/S", "/UPDATE", "/R"];

/// Windows: скачать и проверить установщик, запустить его тихо. Установка
/// в профиль пользователя (currentUser) — без UAC. Приложение сразу после
/// этого выходит (update_install); установщик дождётся выхода — запущенную
/// копию он и сам закрывает — и запустит новую версию. Мост работает из
/// своей папки и обновится при первом старте новой версии.
#[cfg(target_os = "windows")]
pub async fn install(info: &UpdateInfo) -> Result<(), String> {
    let (_, bytes) = download_verified(info).await?;
    let setup = work_dir()?.join("clod-helper-setup.exe");
    std::fs::write(&setup, &bytes).map_err(|e| format!("Не удалось сохранить установщик: {e}"))?;
    std::process::Command::new(&setup)
        .args(NSIS_UPDATE_ARGS)
        .spawn()
        .map_err(|e| format!("Не удалось запустить установщик: {e}"))?;
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

    #[test]
    fn versions_and_sums() {
        assert!(parse_ver("v0.7.0") > parse_ver("0.6.0"));
        assert!(parse_ver("0.6.10") > parse_ver("0.6.9"));
        let sums = "aa11  Clod.Helper_0.6.0_x64-setup.exe\n\
                    0123456789abcdef0123456789abcdef0123456789abcdef0123456789ABCDEF  Clod.Helper_0.6.0_universal.dmg\n";
        assert_eq!(
            expected_sum(sums, "Clod.Helper_0.6.0_universal.dmg").as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(expected_sum(sums, "Clod.Helper_0.6.0_x64-setup.exe"), None, "короткая сумма — не сумма");
        assert_eq!(urlencoding_decode("Clod%20Helper_0.6.0.dmg"), "Clod Helper_0.6.0.dmg");
    }

    /// Суммы CI считает до загрузки: в SHA256SUMS имя с пробелом, а в релизе
    /// GitHub — с точкой (и так же в адресе скачивания).
    #[test]
    fn sums_match_names_renamed_by_github() {
        let exe = "a".repeat(64);
        let dmg = "b".repeat(64);
        let sums = format!(
            "{exe}  Clod Helper_0.7.0_x64-setup.exe\r\n{dmg} *Clod Helper_0.7.0_universal.dmg\r\n"
        );
        assert_eq!(expected_sum(&sums, "Clod.Helper_0.7.0_x64-setup.exe"), Some(exe.clone()));
        assert_eq!(expected_sum(&sums, "Clod.Helper_0.7.0_universal.dmg"), Some(dmg));
        assert_eq!(expected_sum(&sums, "Clod Helper_0.7.0_x64-setup.exe"), Some(exe));
        assert_eq!(expected_sum(&sums, "Clod.Helper_0.7.0_x64-setup.exe.sig"), None);
        assert_eq!(expected_sum(&format!("{}  x.exe", "g".repeat(64)), "x.exe"), None, "не hex");
    }

    fn release(tag: &str, assets: &[&str]) -> Release {
        Release {
            tag_name: tag.into(),
            body: Some("notes".into()),
            draft: false,
            prerelease: false,
            assets: assets
                .iter()
                .map(|n| Asset { name: n.to_string(), browser_download_url: format!("https://x/{n}") })
                .collect(),
        }
    }

    #[test]
    fn picks_installer_of_this_os_with_checksums() {
        let full = ["Clod.Helper_0.8.0_universal.dmg", "Clod.Helper_0.8.0_x64-setup.exe", "SHA256SUMS.txt"];
        let win = pick_update(release("v0.8.0", &full), "0.7.0", "_x64-setup.exe").unwrap();
        assert_eq!(win.version, "0.8.0");
        assert_eq!(win.url, "https://x/Clod.Helper_0.8.0_x64-setup.exe");
        assert_eq!(win.sums_url, "https://x/SHA256SUMS.txt");
        let mac = pick_update(release("v0.8.0", &full), "0.7.0", "_universal.dmg").unwrap();
        assert!(mac.url.ends_with("_universal.dmg"));
        // Не новее, без сумм, без установщика своей ОС — не предлагаем.
        assert!(pick_update(release("v0.7.0", &full), "0.7.0", "_x64-setup.exe").is_none());
        assert!(pick_update(release("v0.8.0", &full[..2]), "0.7.0", "_x64-setup.exe").is_none());
        assert!(pick_update(release("v0.8.0", &[full[0], full[2]]), "0.7.0", "_x64-setup.exe").is_none());
        let mut pre = release("v0.8.0", &full);
        pre.prerelease = true;
        assert!(pick_update(pre, "0.7.0", "_x64-setup.exe").is_none());
    }

    #[test]
    fn windows_update_is_silent_in_place_and_restarts() {
        assert_eq!(NSIS_UPDATE_ARGS, ["/S", "/UPDATE", "/R"]);
    }

    /// Живая проверка: cargo test live_update_check -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_update_check() {
        println!("update → {:?}", check().await);
    }
}
