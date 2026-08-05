use std::sync::Arc;
use std::time::{Duration, Instant};

/// Rotator multi-key untuk Gemini API.
/// Semua key berbagi satu project quota → saat semua rate-limited,
/// set cooldown global 60s (periode reset quota Google AI Studio free tier).
/// TIDAK ada local retry — langsung switch key agar cepat masuk cooldown.
pub struct KeyRotator {
    keys: Vec<String>,
    global_cooldown: Arc<std::sync::Mutex<Option<Instant>>>,
}

impl KeyRotator {
    pub fn new(keys: Vec<String>) -> Self {
        Self {
            keys,
            global_cooldown: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Coba semua key sekali saja. Jika SEMUA 429 → cooldown global 60s.
    pub async fn try_all<F, T>(
        &self,
        mut op: impl FnMut(String) -> F,
    ) -> anyhow::Result<(T, String)>
    where
        F: std::future::Future<Output = anyhow::Result<T>>,
    {
        let len = self.keys.len();
        if len == 0 {
            return Err(anyhow::anyhow!("Tidak ada Gemini API key configured"));
        }

        // Cek cooldown global
        {
            let cd = self.global_cooldown.lock().unwrap();
            if let Some(until) = *cd {
                if Instant::now() < until {
                    let secs = (until - Instant::now()).as_secs() + 1;
                    return Err(anyhow::anyhow!(
                        "Cooldown aktif ({:.0}s). Tunggu {:.0} menit.",
                        secs as f64 / 60.0,
                        secs as f64 / 60.0
                    ));
                }
            }
        }

        // Reset cooldown jika sebelumnya aktif
        {
            let mut cd = self.global_cooldown.lock().unwrap();
            *cd = None;
        }

        let mut last_429_idx = None;
        let mut last_err_msg = String::new();

        for (idx, key) in self.keys.iter().enumerate() {
            match op(key.clone()).await {
                Ok(v) => {
                    eprintln!("✅ Berhasil di Key {} ({:.20}...)", idx, key);
                    return Ok((v, key.clone()));
                }
                Err(ref e) => {
                    let err_str = e.to_string();
                    if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                        last_429_idx = Some(idx);
                        eprintln!("🔑 Key {} rate-limit", idx);
                    } else {
                        eprintln!("⚠️ Key {} error: {}", idx, err_str.chars().take(100).collect::<String>());
                    }
                    last_err_msg = err_str;
                }
            }
        }

        // Semua key kena 429 → cooldown 60 detik (reset period Google)
        if last_429_idx.is_some() {
            let mut cd = self.global_cooldown.lock().unwrap();
            *cd = Some(Instant::now() + Duration::from_secs(60));
            eprintln!(
                "⏳ ALL KEY RATE-LIMITED → cooldown 60s (quota Google AI Studio reset)"
            );
        }

        Err(anyhow::anyhow!(
            "{}",
            if last_429_idx.is_some() {
                format!(
                    "Semua {} key habis quota. Tunggu ~60 detik lalu coba lagi. Error terakhir: {}",
                    len,
                    last_err_msg.chars().take(200).collect::<String>()
                )
            } else {
                format!(
                    "Semua {} key gagal. Error: {}",
                    len,
                    last_err_msg.chars().take(200).collect::<String>()
                )
            }
        ))
    }
}
