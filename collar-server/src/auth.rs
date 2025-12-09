//! Authentication and authorization.

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    async_trait,
    extract::FromRequestParts,
    http::{header::AUTHORIZATION, header::COOKIE, request::Parts, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::{TimeDelta, Utc};
use cookie::Cookie;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

/// Cookie name for JWT token.
pub const AUTH_COOKIE_NAME: &str = "collar_token";

/// JWT claims.
#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: i64,
    pub iat: i64,
}

impl Claims {
    pub fn new(username: &str, expiry_hours: u64) -> Self {
        let now = Utc::now();
        Self {
            sub: username.to_string(),
            iat: now.timestamp(),
            exp: (now + TimeDelta::hours(expiry_hours as i64)).timestamp(),
        }
    }
}

/// Authenticated user extractor.
pub struct AuthUser {
    pub username: String,
}

#[async_trait]
impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Try to get token from cookie first, then Authorization header
        let token = extract_token_from_cookie(&parts.headers)
            .or_else(|| extract_token_from_header(&parts.headers))
            .ok_or(AuthError::MissingToken)?;

        // Decode token
        let token_data = decode::<Claims>(
            &token,
            &DecodingKey::from_secret(state.config.auth.jwt_secret.as_bytes()),
            &Validation::default(),
        )
        .map_err(|_| AuthError::InvalidToken)?;

        Ok(AuthUser {
            username: token_data.claims.sub,
        })
    }
}

/// Extract token from httpOnly cookie.
fn extract_token_from_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    let cookie_header = headers.get(COOKIE)?.to_str().ok()?;

    // Parse cookies and find our auth cookie
    for cookie_str in cookie_header.split(';') {
        if let Ok(cookie) = Cookie::parse(cookie_str.trim()) {
            if cookie.name() == AUTH_COOKIE_NAME {
                return Some(cookie.value().to_string());
            }
        }
    }
    None
}

/// Extract token from Authorization header (fallback for API clients).
fn extract_token_from_header(headers: &axum::http::HeaderMap) -> Option<String> {
    let auth_header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    auth_header.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Authentication error.
#[derive(Debug)]
pub enum AuthError {
    MissingToken,
    InvalidToken,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AuthError::MissingToken => (StatusCode::UNAUTHORIZED, "Missing authentication token"),
            AuthError::InvalidToken => (StatusCode::UNAUTHORIZED, "Invalid token"),
        };

        (status, Json(serde_json::json!({ "error": message }))).into_response()
    }
}

/// Verify password against hash.
pub fn verify_password(password: &str, hash: &str) -> bool {
    let parsed_hash = match PasswordHash::new(hash) {
        Ok(h) => h,
        Err(_) => return false,
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
}

/// Create a JWT token.
pub fn create_token(username: &str, secret: &str, expiry_hours: u64) -> Result<String, String> {
    let claims = Claims::new(username, expiry_hours);
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| e.to_string())
}

/// Create an httpOnly cookie for the auth token.
/// Uses SameSite=None to allow cross-subdomain requests (e.g., collar.example.com -> api.example.com)
pub fn create_auth_cookie(token: &str, expiry_hours: u64) -> String {
    let max_age = expiry_hours * 3600;
    Cookie::build((AUTH_COOKIE_NAME, token))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(cookie::SameSite::None)
        .max_age(cookie::time::Duration::seconds(max_age as i64))
        .to_string()
}

/// Create a cookie that clears the auth token.
pub fn clear_auth_cookie() -> String {
    Cookie::build((AUTH_COOKIE_NAME, ""))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(cookie::SameSite::None)
        .max_age(cookie::time::Duration::ZERO)
        .to_string()
}
