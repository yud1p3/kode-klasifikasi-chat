use std::sync::Arc;

/// Rotator untuk multi-Gemini-API-Key.
/// - Coba semua key berturut-turut secara round-robin.
/// - Track cooldown per key (durasi setelah kena 429).
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

    /// Tandai key tertentu masuk cooldown (setelah kena 429).
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
                if now < until {
                    continue;
                }
            }
            return false; // ada yang non-cooldown
        }
        true
    }

    /// Coba semua key berturut-turut. Switch ke key berikutnya bila kena 429/RESOURCE_EXHAUSTED.
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

        let candidates: Vec<(usize, String)> = (0..len)
            .map(|i| (i, self.keys[i].clone()))
            .collect();

        let mut last_err_msg = String::new();

        for (attempt, (key_idx, key)) in candidates.iter().enumerate() {
            match op(key.clone()).await {
                Ok(v) => return Ok((v, key.clone())),
                Err(ref e) => {
                    let err_str = e.to_string();
                    if err_str.contains("429") || err_str.contains("RESOURCE_EXHAUSTED") {
                        let cooldown_secs = if attempt == 0 { 30 } else { 60 };
                        self.mark_cooldown(key, std::time::Duration::from_secs(cooldown_secs));
                        eprintln!(
                            "🔑 Key {} ({:.20}...) kena rate limit, switch ke key berikutnya...",
                            key_idx, key
                        );
                    } else {
                        eprintln!(
                            "⚠️ Key {} ({:.20}...) error: {}",
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
