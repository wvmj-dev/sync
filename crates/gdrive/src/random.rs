//! Aleatoriedade criptograficamente segura (PKCE, `state`, fronteiras multipart, jitter).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

/// `n` bytes aleatórios em base64url sem padding.
pub fn token(n: usize) -> String {
    let mut buf = vec![0u8; n];
    fill(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

/// Número em [0, 1) para o jitter das retentativas.
pub fn unit() -> f64 {
    let mut buf = [0u8; 8];
    fill(&mut buf);
    (u64::from_le_bytes(buf) >> 11) as f64 / (1u64 << 53) as f64
}

fn fill(buf: &mut [u8]) {
    // Sem fonte de aleatoriedade do SO não há como fazer login com segurança.
    getrandom::fill(buf).expect("fonte de aleatoriedade do sistema indisponível");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_sao_distintos_e_url_safe() {
        let a = token(32);
        assert_ne!(a, token(32));
        assert_eq!(a.len(), 43);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
        let u = unit();
        assert!((0.0..1.0).contains(&u));
    }
}
