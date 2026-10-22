//! Фирменный звук уведомления: короткий двухнотный «динь» (обновление
//! доступно / приложение обновилось).
//!
//! Звук синтезируется на лету в WAV — без файлов-ресурсов и одинаково на
//! обеих ОС. Играет Rust, а не WebView: автоплей в WebView2/WKWebView без
//! жеста пользователя ненадёжен, а окно уведомления клика не ждёт.

use std::sync::OnceLock;

const RATE: u32 = 44_100;

/// Ноты «динь-дон» вверх (до6 → соль6): частота, начало и длительность, с.
const NOTES: [(f32, f32, f32); 2] = [(1046.5, 0.0, 0.42), (1568.0, 0.11, 0.62)];
const VOLUME: f32 = 0.28;

/// WAV (PCM 16 бит, моно) — строится один раз.
pub fn chime_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| {
        let len = NOTES.iter().map(|(_, at, dur)| at + dur).fold(0.0, f32::max);
        let n = (len * RATE as f32) as usize;
        let mut samples = vec![0f32; n];
        for (freq, at, dur) in NOTES {
            let start = (at * RATE as f32) as usize;
            let count = (dur * RATE as f32) as usize;
            for i in 0..count.min(n - start) {
                let t = i as f32 / RATE as f32;
                // Мягкая атака 6 мс (без щелчка) и экспоненциальное затухание,
                // как у колокольчика; вторая гармоника — «стеклянный» призвук.
                let env = (t / 0.006).min(1.0) * (-t * 7.5).exp();
                let w = std::f32::consts::TAU * freq * t;
                samples[start + i] += env * (w.sin() + 0.18 * (2.0 * w).sin());
            }
        }
        let mut pcm = Vec::with_capacity(n * 2);
        for s in samples {
            let v = (s * VOLUME).clamp(-1.0, 1.0);
            pcm.extend_from_slice(&((v * i16::MAX as f32) as i16).to_le_bytes());
        }
        let mut wav = Vec::with_capacity(44 + pcm.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // моно
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        wav
    })
}

/// Сыграть звук асинхронно (не блокирует). Ошибки молча глотаются: звук —
/// украшение, не повод ронять уведомление.
#[cfg(target_os = "windows")]
pub fn chime() {
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    let wav = chime_wav();
    // SAFETY: буфер 'static (OnceLock) и живёт всё время асинхронного
    // проигрывания; SND_MEMORY читает WAV прямо из него.
    unsafe {
        PlaySoundW(wav.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

#[cfg(not(target_os = "windows"))]
pub fn chime() {
    let Some(dir) = dirs::cache_dir().map(|d| d.join("ru.iimperium.clodhelper")) else { return };
    let path = dir.join("chime.wav");
    if !path.exists() {
        let _ = std::fs::create_dir_all(&dir);
        if std::fs::write(&path, chime_wav()).is_err() {
            return;
        }
    }
    let _ = std::process::Command::new("/usr/bin/afplay")
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    #[test]
    fn chime_is_valid_pcm_wav() {
        let wav = super::chime_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..16], b"WAVEfmt ");
        let data = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(wav.len(), 44 + data);
        // Около 0,73 с звука и без перегруза: пик ниже полной шкалы.
        assert!((60_000..70_000).contains(&data), "{data}");
        let peak = wav[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 3000 && peak < i16::MAX as u16, "{peak}");
    }
}
