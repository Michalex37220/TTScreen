//! Extracteurs « à la FastAPI » : corps JSON validé et identifiants de chemin bornés, avec des
//! erreurs 422 au format `{"detail": [{"loc", "msg", "type"}]}`.

use axum::extract::{FromRequest, FromRequestParts, Path, Request};
use axum::http::request::Parts;
use axum::Json;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::error::{ApiError, FieldError};
use crate::schemas::{Validate, MAX_ID};

/// Corps JSON désérialisé puis validé. Toute erreur (JSON invalide, champ manquant, borne dépassée) → 422.
pub struct ApiJson<T>(pub T);

impl<T, S> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned + Validate,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => {
                value.validate().map_err(ApiError::validation)?;
                Ok(ApiJson(value))
            }
            Err(rejection) => {
                if rejection.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                    return Err(ApiError::new(413, "Corps de requête trop volumineux"));
                }
                Err(ApiError::validation(vec![FieldError::new(
                    vec![json!("body")],
                    "json_invalid",
                    rejection.body_text(),
                )]))
            }
        }
    }
}

/// Identifiant de ressource dans le chemin, borné à 1..=2³¹−1 (INTEGER 32 bits côté base :
/// refuser tôt les valeurs hors plage évite un 500).
pub struct Id(pub i32);

fn path_error(kind: &'static str, msg: &str) -> ApiError {
    ApiError::validation(vec![FieldError::new(vec![json!("path"), json!("id")], kind, msg)])
}

pub fn parse_id(raw: &str) -> Result<i32, ApiError> {
    match raw.parse::<i64>() {
        Ok(n) if n < 1 => Err(path_error("greater_than_equal", "Input should be greater than or equal to 1")),
        Ok(n) if n > MAX_ID => Err(path_error("less_than_equal", "Input should be less than or equal to 2147483647")),
        Ok(n) => Ok(n as i32),
        Err(_) => Err(path_error(
            "int_parsing",
            "Input should be a valid integer, unable to parse string as an integer",
        )),
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Id {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Path(raw) = Path::<String>::from_request_parts(parts, state)
            .await
            .map_err(|_| path_error("int_parsing", "Input should be a valid integer"))?;
        parse_id(&raw).map(Id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_must_be_positive_32_bit_integers() {
        assert_eq!(parse_id("1").unwrap(), 1);
        assert_eq!(parse_id("2147483647").unwrap(), i32::MAX);
        for bad in ["0", "-1", "2147483648", "99999999999", "abc", "", "1.5", "1e3", " 1"] {
            let e = parse_id(bad).unwrap_err();
            assert_eq!(e.status.as_u16(), 422, "{bad:?}");
        }
    }

    #[test]
    fn id_error_detail_is_a_list_with_msg() {
        let e = parse_id("0").unwrap_err();
        assert!(e.detail[0]["msg"].as_str().unwrap().contains("greater than or equal to 1"));
        assert_eq!(e.detail[0]["loc"], json!(["path", "id"]));
    }
}
