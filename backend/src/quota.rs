use serde::Serialize;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Pembatas kuota request Gemini (RPM & RPD) agar tidak kena error 429.
///
/// Google menerapkan rate limit PER PROJECT (bukan per key), jadi dihitung
/// global — sesuai komentar di key_rotator.rs ("semua key berbagi satu project
/// quota"). Window RPM di-reset tiap menit; window RPD di-reset tiap tengah
/// malam lokal (sedikit lebih konservatif dari reset "midnight Pacific" Google,
/// karena tengah malam lokal datang belakangan di zona waktu Asia).
#[derive(Clone, Copy, Default)]
struct WindowCounters {
    minute_ts: i64,
    minute_count: u32,
    day_ts: i64,
    day_count: u32,
}

#[derive(Clone, Copy, Serialize)]
pub struct ModelStats {
    pub rpm_used: u32,
    pub rpm_limit: u32,
    pub rpd_used: u32,
    pub rpd_limit: u32,
    pub minute_reset_secs: u64,
    pub day_reset_secs: u64,
}

pub struct ModelQuota {
    limits: (u32, u32), // (rpm, rpd)
    counters: Mutex<WindowCounters>,
}

impl ModelQuota {
    fn now_ts() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn refresh(w: &mut WindowCounters, now: i64) {
        if now / 60 > w.minute_ts / 60 {
            w.minute_ts = now;
            w.minute_count = 0;
        }
        if now / 86400 > w.day_ts / 86400 {
            w.day_ts = now;
            w.day_count = 0;
        }
    }

    /// Ok bila `n` call bisa ditampung; Err((detik sampai reset, alasan)) bila tidak.
    fn can_take(&self, n: u32) -> Result<(), (u64, String)> {
        let mut w = self.counters.lock().unwrap();
        let now = Self::now_ts();
        Self::refresh(&mut w, now);
        if w.minute_count.saturating_add(n) > self.limits.0 {
            let wait = (60 - (now % 60)).max(1) as u64;
            return Err((wait, format!("RPM ({}/{})", w.minute_count, self.limits.0)));
        }
        if w.day_count.saturating_add(n) > self.limits.1 {
            let wait = (86400 - (now % 86400)).max(1) as u64;
            return Err((wait, format!("RPD ({}/{})", w.day_count, self.limits.1)));
        }
        Ok(())
    }

    fn record(&self, n: u32) {
        let mut w = self.counters.lock().unwrap();
        let now = Self::now_ts();
        Self::refresh(&mut w, now);
        w.minute_count = w.minute_count.saturating_add(n);
        w.day_count = w.day_count.saturating_add(n);
    }

    fn stats(&self) -> ModelStats {
        let mut w = self.counters.lock().unwrap();
        let now = Self::now_ts();
        Self::refresh(&mut w, now);
        ModelStats {
            rpm_used: w.minute_count,
            rpm_limit: self.limits.0,
            rpd_used: w.day_count,
            rpd_limit: self.limits.1,
            minute_reset_secs: (60 - (now % 60)) as u64,
            day_reset_secs: (86400 - (now % 86400)) as u64,
        }
    }
}

#[derive(Serialize)]
pub struct QuotaStats {
    pub enabled: bool,
    pub chat: ModelStats,
    pub embed: ModelStats,
    pub overall_pct: f64,
}

pub struct Quota {
    pub enabled: bool,
    pub chat: ModelQuota,
    pub embed: ModelQuota,
}

impl Quota {
    pub fn new(enabled: bool, chat_rpm: u32, chat_rpd: u32, embed_rpm: u32, embed_rpd: u32) -> Self {
        Self {
            enabled,
            chat: ModelQuota { limits: (chat_rpm, chat_rpd), counters: Mutex::new(WindowCounters::default()) },
            embed: ModelQuota { limits: (embed_rpm, embed_rpd), counters: Mutex::new(WindowCounters::default()) },
        }
    }

    /// Cek SEBELUM request: estimasi chat_calls (rerank + select_fungsi) & embed_calls.
    pub fn check(&self, chat_calls: u32, embed_calls: u32) -> Result<(), (u64, String)> {
        if !self.enabled {
            return Ok(());
        }
        if let Err((wait, why)) = self.chat.can_take(chat_calls) {
            return Err((wait, format!("Kuota chat {why}")));
        }
        if let Err((wait, why)) = self.embed.can_take(embed_calls) {
            return Err((wait, format!("Kuota embedding {why}")));
        }
        Ok(())
    }

    pub fn record_chat(&self, n: u32) {
        if self.enabled {
            self.chat.record(n);
        }
    }

    pub fn record_embed(&self, n: u32) {
        if self.enabled {
            self.embed.record(n);
        }
    }

    pub fn stats(&self) -> QuotaStats {
        let chat = self.chat.stats();
        let embed = self.embed.stats();
        let pct = |used: u32, limit: u32| if limit == 0 { 0.0 } else { used as f64 / limit as f64 * 100.0 };
        let overall = pct(chat.rpd_used, chat.rpd_limit)
            .max(pct(embed.rpd_used, embed.rpd_limit))
            .max(pct(chat.rpm_used, chat.rpm_limit))
            .max(pct(embed.rpm_used, embed.rpm_limit));
        QuotaStats { enabled: self.enabled, chat, embed, overall_pct: overall.round() }
    }
}
