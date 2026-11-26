//! Erreurs HTTP au format FastAPI : `{"detail": "message"}` (ou une liste d'objets `msg` pour un 422).

use axum::http::{HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

/// Erreur de validation d'un champ (équivalent d'un élément de `detail` pour un 422 pydantic).
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    pub loc: Vec<Value>,
    pub msg: String,
    pub kind: &'static str,
}

impl FieldError {
    pub fn new(loc: Vec<Value>, kind: &'static str, msg: impl Into<String>) -> Self {
        FieldError { loc, kind, msg: msg.into() }
    }

    /// Champ du corps JSON : `["body", "nom"]`.
    pub fn body(field: &str, kind: &'static str, msg: impl Into<String>) -> Self {
        FieldError::new(vec![json!("body"), json!(field)], kind, msg)
    }

    pub fn to_json(&self) -> Value {
        json!({"type": self.kind, "loc": self.loc, "msg": self.msg})
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub detail: Value,
    pub headers: Vec<(HeaderName, HeaderValue)>,
}

impl ApiError {
    pub fn new(status: u16, detail: impl Into<String>) -> Self {
        ApiError {
            status: StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            detail: Value::String(detail.into()),
            headers: Vec::new(),
        }
    }

    pub fn with_header(mut self, name: &'static str, value: &str) -> Self {
        if let Ok(v) = HeaderValue::from_str(value) {
            self.headers.push((HeaderName::from_static(name), v));
        }
        self
    }

    /// 422 avec une liste d'erreurs de champs (comme FastAPI).
    pub fn validation(errors: Vec<FieldError>) -> Self {
        ApiError {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            detail: Value::Array(errors.iter().map(FieldError::to_json).collect()),
            headers: Vec::new(),
        }
    }

    /// 401 standard des routes d'administration.
    pub fn unauthenticated() -> Self {
        ApiError::new(401, "Authentification requise ou session expirée").with_header("www-authenticate", "Bearer")
    }

    /// 429 avec `Retry-After` (secondes).
    pub fn too_many_requests(retry_after: u64) -> Self {
        ApiError::new(429, "Trop de requêtes, réessayez plus tard.")
            .with_header("retry-after", &retry_after.to_string())
    }

    /// 500 : le détail technique est journalisé, jamais renvoyé au client.
    pub fn internal(err: impl std::fmt::Display) -> Self {
        tracing::error!("erreur interne : {err}");
        ApiError::new(500, "Internal Server Error")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(json!({ "detail": self.detail }))).into_response();
        for (name, value) in self.headers {
            response.headers_mut().insert(name, value);
        }
        response
    }
}

impl From<sea_orm::DbErr> for ApiError {
    fn from(e: sea_orm::DbErr) -> Self {
        ApiError::internal(e)
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::internal(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn api_error_renders_detail_json_and_headers() {
        let resp = ApiError::too_many_requests(7).into_response();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(resp.headers()["retry-after"], "7");
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let v: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["detail"], "Trop de requêtes, réessayez plus tard.");
    }

    #[test]
    fn unauthenticated_carries_www_authenticate() {
        let e = ApiError::unauthenticated();
        assert_eq!(e.status, StatusCode::UNAUTHORIZED);
        assert_eq!(e.headers[0].0.as_str(), "www-authenticate");
    }

    #[test]
    fn validation_error_is_a_list_of_objects_with_msg() {
        let e = ApiError::validation(vec![FieldError::body("name", "string_too_short", "trop court")]);
        assert_eq!(e.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(e.detail[0]["msg"], "trop court");
        assert_eq!(e.detail[0]["loc"], json!(["body", "name"]));
    }

    #[test]
    fn invalid_status_code_degrades_to_500() {
        assert_eq!(ApiError::new(99, "x").status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
