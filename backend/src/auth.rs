use actix_web::HttpRequest;
use anyhow::{bail, Result};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::env;
use std::time::{SystemTime, UNIX_EPOCH};

/// Identitas user yang sudah terverifikasi (dari JWT, atau anonim saat auth nonaktif).
#[derive(Serialize, Clone, Debug)]
pub struct AuthUser {
    pub sub: String,
    pub email: String,
    pub name: String,
}

/// Konfigurasi auth. Bila GOOGLE_CLIENT_ID/GOOGLE_CLIENT_SECRET kosong,
/// auth NONAKTIF → semua endpoint terbuka (mode fallback untuk pengembangan).
pub struct AuthConfig {
    pub enabled: bool,
    pub client_id: String,
    pub client_secret: String,
    /// Daftar redirect URI yang diizinkan (GOOGLE_REDIRECT_URI, comma-separated).
    /// Dipakai untuk mendukung multi-origin: localhost (dev) + domain publik (ngrok).
    pub redirect_uris: Vec<String>,
    pub jwt_secret: String,
    pub jwt_exp_secs: i64,
}

#[derive(Serialize)]
pub struct AuthConfigInfo {
    pub enabled: bool,
    pub client_id: String,
    /// URI pertama (kompatibilitas); gunakan `redirect_uris` untuk daftar lengkap.
    pub redirect_uri: String,
    pub redirect_uris: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    sub: String,
    email: String,
    name: String,
    exp: usize,
}

impl AuthConfig {
    pub fn from_env() -> Self {
        let client_id = env::var("GOOGLE_CLIENT_ID").unwrap_or_default();
        let client_secret = env::var("GOOGLE_CLIENT_SECRET").unwrap_or_default();
        // Bisa lebih dari satu redirect URI (comma-separated): localhost + domain publik
        let redirect_uris: Vec<String> = env::var("GOOGLE_REDIRECT_URI")
            .unwrap_or_else(|_| "http://localhost:5174/auth/callback".into())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let jwt_secret = env::var("JWT_SECRET")
            .unwrap_or_else(|_| "kode-klasifikasi-chat-dev-secret-ganti-di-produksi".into());
        let jwt_exp_secs = env::var("JWT_EXP_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12 * 3600);
        let enabled = !client_id.is_empty() && !client_secret.is_empty();
        Self { enabled, client_id, client_secret, redirect_uris, jwt_secret, jwt_exp_secs }
    }

    /// True bila JWT_SECRET memakai default bawaan (peringatan keamanan).
    pub fn uses_default_secret(&self) -> bool {
        self.jwt_secret == "kode-klasifikasi-chat-dev-secret-ganti-di-produksi"
    }

    pub fn info(&self) -> AuthConfigInfo {
        AuthConfigInfo {
            enabled: self.enabled,
            client_id: self.client_id.clone(),
            redirect_uri: self.redirect_uris.first().cloned().unwrap_or_default(),
            redirect_uris: self.redirect_uris.clone(),
        }
    }

    /// Cek apakah URI termasuk daftar redirect yang diizinkan.
    pub fn is_allowed_redirect(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|r| r == uri)
    }
}

/// Tukar authorization code (dari callback Google) menjadi id_token via PKCE,
/// verifikasi signature RS256 (JWKS Google), lalu kembalikan identitas user.
/// Tukar authorization code → token. `redirect_uri` HARUS sama dengan yang
/// dipakai saat membangun URL login (Google memvalidasinya saat exchange).
pub async fn exchange_code(cfg: &AuthConfig, code: &str, code_verifier: &str, redirect_uri: &str) -> Result<AuthUser> {
    let client = reqwest::Client::new();
    let params = [
        ("client_id", cfg.client_id.clone()),
        ("client_secret", cfg.client_secret.clone()),
        ("code", code.to_string()),
        ("code_verifier", code_verifier.to_string()),
        ("grant_type", "authorization_code".to_string()),
        ("redirect_uri", redirect_uri.to_string()),
    ];
    let resp = client
        .post("https://oauth2.googleapis.com/token")
        .form(&params)
        .send()
        .await?;
    let status = resp.status();
    let json: Value = resp.json().await?;
    if !status.is_success() {
        bail!("Google token exchange gagal ({}): {}", status.as_str(), json);
    }
    let id_token = json["id_token"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Tidak ada id_token dari Google"))?
        .to_string();
    verify_id_token(cfg, &id_token).await
}

/// Verifikasi id_token: ambil JWKS Google, coba verifikasi RS256 dengan tiap kunci.
async fn verify_id_token(cfg: &AuthConfig, id_token: &str) -> Result<AuthUser> {
    let jwks: Value = reqwest::get("https://www.googleapis.com/oauth2/v3/certs")
        .await?
        .json()
        .await?;
    let keys = jwks["keys"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("JWKS Google tidak valid"))?;

    let mut last_err: Option<jsonwebtoken::errors::Error> = None;
    for key in keys {
        let jwk: jsonwebtoken::jwk::Jwk = match serde_json::from_value(key.clone()) {
            Ok(j) => j,
            Err(_) => continue,
        };
        let dk = match DecodingKey::from_jwk(&jwk) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let mut val = Validation::new(Algorithm::RS256);
        val.set_audience(&[&cfg.client_id]);
        val.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
        match decode::<Claims>(id_token, &dk, &val) {
            Ok(data) => {
                let c = data.claims;
                return Ok(AuthUser {
                    sub: c.sub,
                    email: c.email,
                    name: c.name,
                });
            }
            Err(e) => last_err = Some(e),
        }
    }
    bail!("Verifikasi id_token gagal: {:?}", last_err)
}

fn now_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Terbitkan JWT sesi (HS256, secret server).
pub fn issue_token(cfg: &AuthConfig, user: &AuthUser) -> Result<String> {
    let claims = Claims {
        sub: user.sub.clone(),
        email: user.email.clone(),
        name: user.name.clone(),
        exp: (now_ts() + cfg.jwt_exp_secs) as usize,
    };
    Ok(encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(cfg.jwt_secret.as_bytes()),
    )?)
}

/// Verifikasi JWT → identitas user (None bila tidak valid/kedaluwarsa).
pub fn verify_token(cfg: &AuthConfig, token: &str) -> Option<AuthUser> {
    let mut val = Validation::new(Algorithm::HS256);
    val.validate_exp = true;
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(cfg.jwt_secret.as_bytes()),
        &val,
    )
    .ok()?;
    Some(AuthUser {
        sub: data.claims.sub,
        email: data.claims.email,
        name: data.claims.name,
    })
}

/// Ambil token Bearer dari header Authorization.
pub fn bearer_token(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
}

/// Wajib login: bila auth aktif, kembalikan Err(401) saat token tidak valid.
/// Bila auth nonaktif → user anonim (mode fallback).
pub fn require_user(req: &HttpRequest, cfg: &AuthConfig) -> Result<AuthUser, actix_web::HttpResponse> {
    if !cfg.enabled {
        return Ok(AuthUser { sub: "anon".into(), email: "anon@local".into(), name: "Anonim".into() });
    }
    match bearer_token(req).and_then(|t| verify_token(cfg, &t)) {
        Some(u) => Ok(u),
        None => Err(actix_web::HttpResponse::Unauthorized().json(serde_json::json!({
            "error": "Silakan login terlebih dahulu"
        }))),
    }
}
