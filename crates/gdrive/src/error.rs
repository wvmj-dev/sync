//! Classificação dos erros da Drive API em erros do núcleo (RN-96 a RN-98).
//! Referência: <https://developers.google.com/workspace/drive/api/guides/handle-errors>

use std::time::Duration;

use serde::Deserialize;
use syncly_core::provider::ProviderError;

#[derive(Deserialize)]
struct Envelope {
    error: Body,
}

#[derive(Deserialize)]
struct Body {
    #[serde(default)]
    message: String,
    #[serde(default)]
    errors: Vec<Detail>,
}

#[derive(Deserialize)]
struct Detail {
    #[serde(default)]
    reason: String,
}

/// Extrai (`reason`, `message`) do corpo JSON de erro do Google.
pub fn parse_body(body: &str) -> (Option<String>, String) {
    match serde_json::from_str::<Envelope>(body) {
        Ok(env) => (
            env.error
                .errors
                .into_iter()
                .map(|d| d.reason)
                .find(|r| !r.is_empty()),
            env.error.message,
        ),
        Err(_) => (None, String::new()),
    }
}

pub fn classify(
    status: u16,
    reason: Option<&str>,
    message: &str,
    retry_after: Option<Duration>,
) -> ProviderError {
    let msg = || {
        if message.is_empty() {
            format!("HTTP {status}")
        } else {
            message.to_string()
        }
    };
    match (status, reason.unwrap_or_default()) {
        (429, _)
        | (403, "rateLimitExceeded" | "userRateLimitExceeded" | "sharingRateLimitExceeded") => {
            ProviderError::RateLimited { retry_after }
        }
        (401, _) => ProviderError::AuthExpired,
        (403, "storageQuotaExceeded" | "activeItemCreationLimitExceeded") => {
            ProviderError::StorageFull
        }
        (403, "dailyLimitExceeded") => ProviderError::DailyLimit,
        (403, _) => ProviderError::PermissionDenied(msg()),
        (404, _) => ProviderError::NotFound(msg()),
        (400, _) => ProviderError::BadRequest(msg()),
        (408, _) => ProviderError::Network("tempo esgotado".into()),
        (500..=599, _) => ProviderError::Server {
            status,
            retry_after,
        },
        _ => ProviderError::Other(msg()),
    }
}

/// Erros do endpoint de token. `invalid_grant` = token revogado/expirado (SEG-07).
pub fn classify_token_error(body: &str) -> ProviderError {
    #[derive(Deserialize)]
    struct TokenError {
        error: String,
    }
    match serde_json::from_str::<TokenError>(body) {
        Ok(e) if e.error == "invalid_grant" => ProviderError::AuthExpired,
        Ok(e) => ProviderError::Other(format!("falha na autenticação: {}", e.error)),
        Err(_) => ProviderError::Other("resposta inválida do servidor de autenticação".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use syncly_core::provider::ErrorScope;

    fn scope(status: u16, reason: &str) -> ErrorScope {
        classify(status, Some(reason), "", None).scope()
    }

    #[test]
    fn le_motivo_do_corpo() {
        let body = r#"{"error":{"code":403,"message":"User rate limit exceeded.",
            "errors":[{"domain":"usageLimits","reason":"userRateLimitExceeded","message":"x"}]}}"#;
        let (reason, message) = parse_body(body);
        assert_eq!(reason.as_deref(), Some("userRateLimitExceeded"));
        assert_eq!(message, "User rate limit exceeded.");
        assert_eq!(parse_body("<html>"), (None, String::new()));
    }

    #[test]
    fn rn96_transitorios() {
        assert_eq!(scope(429, ""), ErrorScope::Transient);
        assert_eq!(scope(403, "userRateLimitExceeded"), ErrorScope::Transient);
        assert_eq!(scope(403, "rateLimitExceeded"), ErrorScope::Transient);
        assert_eq!(scope(503, ""), ErrorScope::Transient);
    }

    #[test]
    fn rn98_afetam_a_conta() {
        assert_eq!(scope(401, "authError"), ErrorScope::Account);
        assert_eq!(scope(403, "storageQuotaExceeded"), ErrorScope::Account);
        assert_eq!(scope(403, "dailyLimitExceeded"), ErrorScope::Account);
    }

    #[test]
    fn rn97_afetam_o_item() {
        assert_eq!(scope(403, "insufficientFilePermissions"), ErrorScope::Item);
        assert_eq!(scope(404, "notFound"), ErrorScope::Item);
        assert_eq!(scope(400, "badRequest"), ErrorScope::Item);
    }

    #[test]
    fn seg07_invalid_grant_marca_reautenticacao() {
        assert!(matches!(
            classify_token_error(r#"{"error":"invalid_grant"}"#),
            ProviderError::AuthExpired
        ));
    }
}
