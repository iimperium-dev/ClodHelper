//! Фирменный звук уведомления «Тук-тук» (обновление доступно / приложение
//! обновилось): две деревянные ноты вверх (соль4 → до5), у каждой —
//! короткий чёткий щелчок в начале и мягкое тёплое тело под фильтром.
//! Выбран владельцем из прослушанных вариантов: щелчок не даёт уведомлению
//! потеряться, а мягкое тело не режет слух.
//!
//! Звук синтезируется на лету в WAV — без файлов-ресурсов и одинаково на
//! обеих ОС. Играет Rust, а не WebView: автоплей в WebView2/WKWebView без
//! жеста пользователя ненадёжен, а окно уведомления клика не ждёт.

use std::sync::OnceLock;

const RATE: u32 = 44_100;
const LEN_S: f32 = 1.0;
const VOLUME: f32 = 0.28;

/// Мягкие ноты: частота, начало (с), скорость затухания. У каждой — основной
/// тон и «деревянный» призвук ×4, гаснущий быстрее.
const NOTES: [(f32, f32, f32); 2] = [(392.0, 0.0, 9.0), (523.25, 0.12, 6.0)];
/// Щелчки: центр полосы шума (Гц) и начало (с).
const CLICKS: [(f32, f32); 2] = [(2600.0, 0.0), (3000.0, 0.12)];
/// Срез мягкого слоя (как lowpass 2 кГц в превью).
const SOFT_CUTOFF: f32 = 2000.0;

/// Биквад RBJ (lowpass / bandpass) — та же фильтрация, что в превью WebAudio.
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn new(kind: &str, f: f32, q: f32) -> Self {
        let w = std::f32::consts::TAU * f / RATE as f32;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if kind == "lowpass" {
            [(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0]
        } else {
            [alpha, 0.0, -alpha]
        };
        Self { b: b.map(|x| x / a0), a: [-2.0 * cos / a0, (1.0 - alpha) / a0], z: [0.0; 2] }
    }
    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Огибающая: линейная атака, затем экспонента (как setTargetAtTime).
fn env(t: f32, attack: f32, k: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < attack {
        t / attack
    } else {
        (-(t - attack) * k).exp()
    }
}

/// WAV (PCM 16 бит, моно) — строится один раз.
pub fn chime_wav() -> &'static [u8] {
    static WAV: OnceLock<Vec<u8>> = OnceLock::new();
    WAV.get_or_init(|| {
        let n = (LEN_S * RATE as f32) as usize;
        let mut samples = vec![0f32; n];
        // Мягкий слой: синусы через lowpass.
        let mut lp = Biquad::new("lowpass", SOFT_CUTOFF, 0.5);
        for (i, out) in samples.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let mut soft = 0.0;
            for (f, at, k) in NOTES {
                let tt = t - at;
                if tt < 0.0 {
                    continue;
                }
                let w = std::f32::consts::TAU * f * tt;
                soft += env(tt, 0.004, k) * w.sin() + 0.18 * env(tt, 0.004, 30.0) * (4.0 * w).sin();
            }
            *out = lp.run(soft);
        }
        // Щелчки: короткие вспышки шума в полосе — без фильтра мягкого слоя.
        let mut seed: u32 = 0x1234_5678;
        for (f, at) in CLICKS {
            let mut bp = Biquad::new("bandpass", f, 1.2);
            let start = (at * RATE as f32) as usize;
            for i in 0..(0.08 * RATE as f32) as usize {
                let Some(out) = samples.get_mut(start + i) else { break };
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                *out += 0.35 * env(i as f32 / RATE as f32, 0.001, 110.0) * bp.run(noise);
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
        // Секунда звука и без перегруза: пик ниже полной шкалы.
        assert_eq!(data, 88_200);
        let peak = wav[44..].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap();
        assert!(peak > 3000 && peak < i16::MAX as u16, "{peak}");
    }

    /// Послушать: CHIME_OUT=путь.wav cargo test save_chime -- --ignored
    #[test]
    #[ignore]
    fn save_chime() {
        std::fs::write(std::env::var("CHIME_OUT").unwrap(), super::chime_wav()).unwrap();
    }
}
