//! Política de novas tentativas (RN-96, SEG-41): backoff exponencial com jitter,
//! respeitando `Retry-After`. Gentil com a API — nunca martela o servidor.

use std::time::Duration;

use crate::provider::{ErrorScope, ProviderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 6,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(64),
        }
    }
}

impl RetryPolicy {
    /// Espera antes da próxima tentativa, ou `None` se não deve tentar de novo.
    /// `attempt` é o número da tentativa que acabou de falhar (começa em 1);
    /// `jitter` ∈ [0, 1] vem do chamador (mantém a função pura e testável).
    pub fn next_delay(&self, attempt: u32, err: &ProviderError, jitter: f64) -> Option<Duration> {
        if err.scope() != ErrorScope::Transient || attempt >= self.max_attempts {
            return None;
        }
        let exp = self
            .base_delay
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
        let cap = exp.min(self.max_delay);
        // "Equal jitter": metade fixa + metade aleatória — nunca tenta imediatamente.
        let delay = cap / 2 + cap.mul_f64(jitter.clamp(0.0, 1.0) / 2.0);
        Some(match err.retry_after() {
            Some(server) => delay.max(server),
            None => delay,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cresce_exponencialmente_ate_o_teto() {
        let p = RetryPolicy::default();
        let net = ProviderError::Network("x".into());
        assert_eq!(p.next_delay(1, &net, 1.0), Some(Duration::from_secs(1)));
        assert_eq!(p.next_delay(3, &net, 1.0), Some(Duration::from_secs(4)));
        assert_eq!(p.next_delay(3, &net, 0.0), Some(Duration::from_secs(2)));
        let p = RetryPolicy {
            max_attempts: 20,
            ..p
        };
        assert_eq!(p.next_delay(15, &net, 1.0), Some(Duration::from_secs(64)));
    }

    #[test]
    fn respeita_retry_after_e_limite_de_tentativas() {
        let p = RetryPolicy::default();
        let limited = ProviderError::RateLimited {
            retry_after: Some(Duration::from_secs(30)),
        };
        assert_eq!(
            p.next_delay(1, &limited, 0.0),
            Some(Duration::from_secs(30))
        );
        assert_eq!(p.next_delay(6, &limited, 0.0), None);
    }

    #[test]
    fn rn97_rn98_erros_nao_transitorios_nao_repetem() {
        let p = RetryPolicy::default();
        assert_eq!(p.next_delay(1, &ProviderError::AuthExpired, 0.5), None);
        assert_eq!(p.next_delay(1, &ProviderError::StorageFull, 0.5), None);
        assert_eq!(
            p.next_delay(1, &ProviderError::NotFound("x".into()), 0.5),
            None
        );
    }
}
