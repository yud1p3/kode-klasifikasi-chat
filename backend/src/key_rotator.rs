use std::sync::Arc;

/// Rotator untuk multi-Gemini-API-Key.
/// - Coba semua key secara round-robin (sequential dari index 0).
/// - Cooldown pendek (10s) karena Google token bucket reset cepat (~1 menit).
pub struct KeyRotator {
    keys: Vec<String>,
    cooldowns: Arc<Vec<std::sync::Mutex<Option<std::time::Instant>>>>,
}

impl KeyRotator {
    pub fn new(keys: Vec<String>) -> Self {
        let len = keys.len();
        Self {
            keys,
            cooldowns: Arc::new((0..len).map(|_| std::sync::Mutex::new(None)).collect()),
        }
    }

    /// Tandai key masuk cooldown setelah kena 429/RESOURCE_EXHAUSTED.
    pub fn mark_cooldown(&self, key: &str, duration: std::time::Duration) {
        for (i, k) in self.keys.iter().enumerate() {
            if k == key {
                let mut cd = self.cooldowns[i].lock().unwrap();
                *cd = Some(std::time::Instant::now() + duration);
                break;
            }
        }
    }

    /// Cek apakah semua key sedang cooldown.
    #[allow(dead_code)]
    pub fn all_cooldowned(&self) -> bool {
        let now = std::time::Instant::now();
        for i in 0..self.keys.len() {
            let cd = self.cooldowns[i].lock().unwrap();
            if let Some(until) = *cd {
                if now < until { continue; }
            }
            return false;
        }
        true
    }

    /// Coba semua key berturut-turut dengan auto-switch pada 429.
    /// Tiap key mendapat 1x kesempatan (tanpa retry lokal berulang)
    /// karena kita punya banyak key — lebih efisien coba key baru daripada
    /// retry berkali-kali di key yang sama yang sedang rate-limited.
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
        if len == 1 {
            let result = op(self.keys[0].clone()).await;
            return result.map(|v| (v, self.keys[0].clone()));
        }

        // Iterasi semua key mulai dari index 0
        let candidates: Vec<(usize, String)> = (0..len)
            .map(|i| (i, self.keys[i].clone()))
            .collect();

        let mut last_err_msg = String::new();

        for (_attempt, (key_idx, key)) in candidates.iter().enumerate() {
            match op(key.clone()).await {
                Ok(v) => return Ok((v, key.clone())),
                Err(ref e) => {
                    let err_str = e.to_string();
                    if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                        // Cooldown pendek: cukup tunggu refresh token bucket (~10 detik)
                        self.mark_cooldown(
                            key,
                            std::time::Duration::from_secs(10),
                        );
                        let next = if key_idx + 1 < len { key_idx + 1 } else { 0 };
                        eprintln!(
                            "🔑 Key {} ({:.20}...) rate limit → switch ke Key {}",
                            key_idx, key, next
                        );
                    } else {
                        eprintln!(
                            "⚠️ Key {} ({:.20}...) error lain: {}",
                            key_idx, key, err_str
                        );
                    }
                    last_err_msg = err_str;
                }
            }
        }

        Err(anyhow::anyhow!(
            "Semua {} key gagal. Terakhir: {}",
            len,
            last_err_msg
        ))
    }
}
