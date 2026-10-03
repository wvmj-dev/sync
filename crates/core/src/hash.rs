//! Hash de conteúdo. Cada provedor declara o algoritmo que o servidor informa
//! (Drive: MD5); o scanner calcula o mesmo localmente para comparar sem baixar nada.

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;

use md5::{Digest, Md5};

const BUF_SIZE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashKind {
    Md5,
}

/// Acumulador incremental, para calcular o hash enquanto os bytes trafegam.
pub enum ContentHasher {
    Md5(Md5),
}

impl ContentHasher {
    pub fn new(kind: HashKind) -> Self {
        match kind {
            HashKind::Md5 => Self::Md5(Md5::new()),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        match self {
            Self::Md5(h) => h.update(data),
        }
    }

    pub fn finish(self) -> String {
        match self {
            Self::Md5(h) => hex::encode(h.finalize()),
        }
    }
}

/// Hash de um arquivo inteiro, em streaming (memória constante).
pub fn hash_file(path: &Path, kind: HashKind) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = ContentHasher::new(kind);
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish())
}

/// Copia `src` para `dest` calculando o hash na mesma leitura. Retorna (bytes, hash).
/// Faz `fsync` no destino antes de retornar (SEG-14).
pub fn copy_hashing(src: &Path, dest: &mut File, kind: HashKind) -> io::Result<(u64, String)> {
    let mut input = File::open(src)?;
    let mut hasher = ContentHasher::new(kind);
    let mut buf = vec![0u8; BUF_SIZE];
    let mut total = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        dest.write_all(&buf[..n])?;
        total += n as u64;
    }
    dest.sync_all()?;
    Ok((total, hasher.finish()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_de_arquivo_conhecido() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            hash_file(&path, HashKind::Md5).unwrap(),
            "900150983cd24fb0d6963f7d28e17f72"
        );
    }
}
