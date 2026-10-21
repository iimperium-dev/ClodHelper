//! Обновление Clod Helper без подписи Apple (macOS).
//!
//! Проверка: последний релиз GitHub (iimperium-dev/Clod-Helper-v1). Если он
//! новее — по кнопке скачиваем `*_universal.dmg`, сверяем SHA-256 с
//! `SHA256SUMS.txt` того же релиза (CI публикует его рядом), ставим новую
//! версию на место текущей и перезапускаемся. Старая версия уходит в
//! ~/Library/Application Support/ClodHelper-backup — откат вручную.

use serde::{Deserialize, Serialize};

const REPO: &str = "iimperium-dev/Clod-Helper-v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub dmg_url: String,
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
    if r.draft || r.prerelease {
        return Ok(None);
    }
    let (Some(latest), Some(current)) = (parse_ver(&r.tag_name), parse_ver(env!("CARGO_PKG_VERSION"))) else {
        return Ok(None);
    };
    if latest <= current {
        return Ok(None);
    }
    let find = |pred: &dyn Fn(&str) -> bool| r.assets.iter().find(|a| pred(&a.name)).map(|a| a.browser_download_url.clone());
    let (Some(dmg_url), Some(sums_url)) = (
        find(&|n| n.ends_with("_universal.dmg")),
        find(&|n| n == "SHA256SUMS.txt"),
    ) else {
        // Без контрольной суммы не ставим — нечем проверить целостность.
        return Ok(None);
    };
    let notes: String = r.body.unwrap_or_default().chars().take(600).collect();
    Ok(Some(UpdateInfo {
        version: r.tag_name.trim_start_matches('v').to_string(),
        notes,
        dmg_url,
        sums_url,
    }))
}

/// Сумма файла из SHA256SUMS ("<hex>  <имя>").
pub fn expected_sum(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut p = l.split_whitespace();
        let (sum, name) = (p.next()?, p.next()?);
        (name.trim_start_matches('*') == file && sum.len() == 64).then(|| sum.to_lowercase())
    })
}

/// Скачать, проверить, поставить и перезапуститься (после вызова приложение
/// должно завершиться — новую версию запустит фоновый скрипт).
#[cfg(not(target_os = "windows"))]
pub async fn install(info: &UpdateInfo) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::path::PathBuf;

    // Где стоит текущее приложение: …/Clod Helper.app/Contents/MacOS/clod-helper
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let bundle: PathBuf = exe
        .ancestors()
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .map(PathBuf::from)
        .ok_or("Обновление работает только для установленного приложения (.app)")?;

    let c = client()?;
    let sums = c
        .get(&info.sums_url)
        .send()
        .await
        .map_err(|e| format!("Контрольные суммы: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Контрольные суммы: {e}"))?;
    let file = info.dmg_url.rsplit('/').next().unwrap_or_default().to_string();
    let file = urlencoding_decode(&file);
    let want = expected_sum(&sums, &file).ok_or("В релизе нет контрольной суммы для .dmg")?;
    let bytes = c
        .get(&info.dmg_url)
        .timeout(std::time::Duration::from_secs(300))
        .send()
        .await
        .map_err(|e| format!("Загрузка: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("Загрузка: {e}"))?;
    let got = format!("{:x}", Sha256::digest(&bytes));
    if got != want {
        return Err("Файл обновления повреждён (контрольная сумма не совпала) — не ставлю".into());
    }

    let work = dirs::cache_dir()
        .ok_or("Нет папки кэша")?
        .join("ru.iimperium.clodhelper/update");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
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

#[cfg(target_os = "windows")]
pub async fn install(_info: &UpdateInfo) -> Result<(), String> {
    Err("Скачайте установщик со страницы релиза".into())
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

    /// Живая проверка: cargo test live_update_check -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_update_check() {
        println!("update → {:?}", check().await);
    }
}
