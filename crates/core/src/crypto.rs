//! Criptografia das versões (RN-70 a RN-74, SEG-60, SEG-61): `age` com chave X25519
//! (ChaCha20-Poly1305 no conteúdo). Nada "caseiro" — este módulo só junta as peças do crate
//! `age`. A chave pública cifra (backup automático sem segredo, RN-73); a privada só decifra.

use std::fmt;
use std::io::{self, Read, Write};
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

/// Extensão acrescentada ao arquivo cifrado (`.tar.zst.age`, RN-63).
pub const EXTENSION: &str = "age";
/// Primeira linha de todo arquivo `age` (serve para reconhecer um arquivo cifrado).
pub const MAGIC: &[u8] = b"age-encryption.org/v1";
/// Arquivo de recuperação maior que isso não é uma chave.
pub const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;

/// Chave pública (`age1…`). Pode ficar no `config.toml` (RN-71).
#[derive(Clone)]
pub struct PublicKey(age::x25519::Recipient);

impl PublicKey {
    pub fn parse(text: &str) -> Result<Self, String> {
        age::x25519::Recipient::from_str(text.trim())
            .map(Self)
            .map_err(|_| "chave pública inválida (esperado age1…)".to_string())
    }

    /// RN-74: identifica a chave sem revelar nada — 16 dígitos do SHA-256 da chave pública.
    pub fn fingerprint(&self) -> String {
        let digest = Sha256::digest(self.0.to_string().as_bytes());
        hex::encode(&digest[..8])
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.fingerprint())
    }
}

/// Chave privada. Fica no keyring e no arquivo de recuperação do usuário — nunca em log,
/// DTO ou mensagem de erro (SEG-04): o `Debug` mostra só a impressão digital.
pub struct SecretKey(age::x25519::Identity);

impl SecretKey {
    pub fn generate() -> Self {
        Self(age::x25519::Identity::generate())
    }

    /// Uma chave `AGE-SECRET-KEY-1…`.
    pub fn parse(text: &str) -> Result<Self, String> {
        age::x25519::Identity::from_str(text.trim())
            .map(Self)
            .map_err(|_| "chave de recuperação inválida".to_string())
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.to_public())
    }

    /// O segredo em texto — só para gravar no keyring ou no arquivo de recuperação.
    pub fn expose(&self) -> String {
        self.0.to_string().expose_secret().to_string()
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretKey({})", self.public().fingerprint())
    }
}

/// RN-72: o arquivo de recuperação. Formato de identidade do `age`, que o próprio programa
/// `age` entende (`age -d -i <arquivo> versão.tar.zst.age`) — dá para recuperar as versões
/// mesmo sem o Syncly.
pub fn recovery_file(key: &SecretKey, created: DateTime<Utc>) -> String {
    let public = key.public();
    format!(
        "# Syncly — chave de recuperação das versões criptografadas.\n\
         # Sem este arquivo, as versões criptografadas com esta chave NÃO podem ser abertas.\n\
         # Guarde-o fora deste computador (pendrive, gerenciador de senhas) e não o compartilhe.\n\
         # Abrir sem o Syncly: age -d -i <este arquivo> <versão>.tar.zst.age > versão.tar.zst\n\
         # criada: {}\n\
         # impressão digital: {}\n\
         # chave pública: {public}\n\
         {}\n",
        created.format("%Y-%m-%d %H:%M UTC"),
        public.fingerprint(),
        key.expose(),
    )
}

/// Lê um arquivo de recuperação (ou qualquer arquivo de identidade do `age` com uma única
/// chave X25519): linhas em branco e comentários `#` são ignorados.
pub fn parse_recovery_file(text: &str) -> Result<SecretKey, String> {
    let keys: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    match keys.as_slice() {
        [one] => SecretKey::parse(one),
        [] => Err("o arquivo não tem nenhuma chave".into()),
        _ => Err("o arquivo tem mais de uma chave — use o arquivo de recuperação do Syncly".into()),
    }
}

/// `nome.tar.zst.age` → `Some("nome.tar.zst")`; sem `.age`, `None`.
pub fn strip_extension(file_name: &str) -> Option<&str> {
    let suffix = format!(".{EXTENSION}");
    let cut = file_name.len().checked_sub(suffix.len())?;
    file_name
        .get(cut..)
        .filter(|s| s.eq_ignore_ascii_case(&suffix))
        .map(|_| &file_name[..cut])
}

/// Cifra tudo o que for escrito. **Chame `finish()`** no fim, senão o arquivo fica truncado.
pub fn encrypt_writer<W: Write>(
    key: &PublicKey,
    out: W,
) -> io::Result<age::stream::StreamWriter<W>> {
    let recipient: &dyn age::Recipient = &key.0;
    age::Encryptor::with_recipients(std::iter::once(recipient))
        .map_err(io::Error::other)?
        .wrap_output(out)
}

/// Decifra com a primeira chave que servir. Conteúdo adulterado vira erro de leitura
/// mais adiante (cada bloco é autenticado) — nunca dado errado em silêncio.
pub fn decrypt_reader<R: Read>(
    keys: &[SecretKey],
    input: R,
) -> Result<age::stream::StreamReader<R>, String> {
    let decryptor = age::Decryptor::new(input).map_err(|e| match e {
        age::DecryptError::Io(e) => format!("não foi possível ler o arquivo: {e}"),
        _ => "o arquivo não é uma versão criptografada válida".to_string(),
    })?;
    if keys.is_empty() {
        return Err(no_key());
    }
    let identities = keys.iter().map(|k| &k.0 as &dyn age::Identity);
    decryptor.decrypt(identities).map_err(|e| match e {
        age::DecryptError::NoMatchingKeys => no_key(),
        _ => "o arquivo criptografado está corrompido ou foi alterado".to_string(),
    })
}

fn no_key() -> String {
    "nenhuma chave deste computador abre esta versão — importe a chave de recuperação \
     (Preferências → Criptografia)"
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seg61_chave_privada_nunca_aparece_no_debug() {
        let key = SecretKey::generate();
        let shown = format!("{key:?} {:?}", key.public());
        assert!(!shown.contains(&key.expose()));
        assert!(shown.contains(&key.public().fingerprint()));
        assert!(key.expose().starts_with("AGE-SECRET-KEY-1"));
        assert!(key.public().to_string().starts_with("age1"));
    }

    #[test]
    fn rn74_impressao_digital_estavel_e_curta() {
        let key = SecretKey::generate();
        let fp = key.public().fingerprint();
        assert_eq!(fp.len(), 16);
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
        let again = PublicKey::parse(&key.public().to_string()).unwrap();
        assert_eq!(again.fingerprint(), fp);
        assert_ne!(SecretKey::generate().public().fingerprint(), fp);
    }

    #[test]
    fn rn72_arquivo_de_recuperacao_volta_a_mesma_chave() {
        let key = SecretKey::generate();
        let file = recovery_file(&key, Utc::now());
        assert!(file.contains("NÃO podem ser abertas"));
        let back = parse_recovery_file(&file).unwrap();
        assert_eq!(back.expose(), key.expose());
        assert!(parse_recovery_file("# só comentário\n").is_err());
        let two = format!("{}\n{}\n", key.expose(), SecretKey::generate().expose());
        assert!(parse_recovery_file(&two).is_err());
        assert!(parse_recovery_file("AGE-SECRET-KEY-1INVALIDA").is_err());
    }

    #[test]
    fn cifra_e_decifra_so_com_a_chave_certa() {
        let key = SecretKey::generate();
        let mut sealed = Vec::new();
        let mut w = encrypt_writer(&key.public(), &mut sealed).unwrap();
        w.write_all(b"conteudo secreto").unwrap();
        w.finish().unwrap();
        assert!(sealed.starts_with(MAGIC));
        assert!(!sealed.windows(16).any(|w| w == b"conteudo secreto"));

        let mut plain = String::new();
        decrypt_reader(&[key], sealed.as_slice())
            .unwrap()
            .read_to_string(&mut plain)
            .unwrap();
        assert_eq!(plain, "conteudo secreto");

        let other = SecretKey::generate();
        let err = decrypt_reader(&[other], sealed.as_slice()).err().unwrap();
        assert!(err.contains("importe a chave"), "{err}");
        assert!(decrypt_reader(&[], sealed.as_slice()).is_err());
        assert!(decrypt_reader(&[SecretKey::generate()], &b"texto comum"[..]).is_err());
    }

    #[test]
    fn reconhece_a_extensao() {
        assert_eq!(strip_extension("v.tar.zst.age"), Some("v.tar.zst"));
        assert_eq!(strip_extension("V.TAR.ZST.AGE"), Some("V.TAR.ZST"));
        assert_eq!(strip_extension("v.tar.zst"), None);
        assert_eq!(strip_extension("age"), None);
        assert_eq!(strip_extension("ção.age"), Some("ção"));
    }
}
