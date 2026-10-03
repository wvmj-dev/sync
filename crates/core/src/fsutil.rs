//! Utilitários de sistema de arquivos com foco em segurança (SEG-14, SEG-53).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::{CoreResult, IoContext};

/// Cria o diretório (e os pais) com permissão `0700`; corrige se já existir mais aberto.
pub fn ensure_private_dir(path: &Path) -> CoreResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .at(path)?;
        let mode = fs::metadata(path).at(path)?.permissions().mode() & 0o777;
        if mode != 0o700 {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).at(path)?;
        }
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path).at(path)?;
    Ok(())
}

/// Abre um arquivo novo (falha se existir) com permissão `0600`.
pub fn create_private_file(path: &Path) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Escrita atômica: arquivo temporário no mesmo diretório → `fsync` → `rename`.
/// Leitores concorrentes (GUI e CLI) veem o conteúdo antigo ou o novo, nunca pela metade.
pub fn write_atomic(path: &Path, contents: &[u8]) -> CoreResult<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{name}.tmp-{}", uuid::Uuid::new_v4()));

    let result = (|| {
        let mut file = create_private_file(&tmp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&tmp, path)?;
        // Garante que o rename sobreviva a uma queda de energia.
        #[cfg(unix)]
        File::open(dir)?.sync_all()?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.at(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escrita_atomica_substitui_conteudo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        write_atomic(&path, b"um").unwrap();
        write_atomic(&path, b"dois").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "dois");
        // Nenhum temporário sobrando.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn arquivos_e_pastas_privados() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("a/b");
        ensure_private_dir(&sub).unwrap();
        assert_eq!(
            fs::metadata(&sub).unwrap().permissions().mode() & 0o777,
            0o700
        );

        let file = sub.join("x");
        write_atomic(&file, b"x").unwrap();
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
