use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header::COOKIE},
    middleware::Next,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};

use crate::app_state::AppState;

pub const SESSION_COOKIE: &str = "eyes_on_me_session";

pub fn derive_session_token(dashboard_token: &str) -> String {
    let digest = Sha256::digest(format!("eyes-on-me-dashboard-session:{dashboard_token}"));
    format!("{digest:x}")
}

pub fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

pub fn session_cookie_value(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| (name == SESSION_COOKIE).then_some(value))
}

pub fn login_cookie(session_token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SESSION_COOKIE}={session_token}; Path=/; HttpOnly; SameSite=Strict; Max-Age=604800{secure}"
    )
}

pub fn clear_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
}

pub async fn require_dashboard_auth(
    State(state): State<Arc<AppState>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if !state.dashboard_auth_required() {
        return next.run(request).await;
    }

    let authorized = session_cookie_value(request.headers())
        .map(|value| state.is_dashboard_session_valid(value))
        .unwrap_or(false);
    if authorized {
        next.run(request).await
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header::COOKIE};

    use super::{SESSION_COOKIE, constant_time_eq, derive_session_token, session_cookie_value};

    #[test]
    fn derives_stable_non_secret_session_token() {
        let session = derive_session_token("dashboard-secret");
        assert_eq!(session.len(), 64);
        assert!(!session.contains("dashboard-secret"));
        assert_eq!(session, derive_session_token("dashboard-secret"));
    }

    #[test]
    fn compares_secrets_and_reads_cookie() {
        assert!(constant_time_eq("same", "same"));
        assert!(!constant_time_eq("same", "different"));

        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_str(&format!("theme=dark; {SESSION_COOKIE}=session-1")).unwrap(),
        );
        assert_eq!(session_cookie_value(&headers), Some("session-1"));
    }
}
