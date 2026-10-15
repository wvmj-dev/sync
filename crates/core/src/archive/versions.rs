//! Versões de tarefas compactadas (RN-50, RN-59): nome com data e "impressão digital"
//! do conteúdo das fontes.

use chrono::NaiveDateTime;

use crate::domain::rel_path::validate_name;
use crate::domain::{ArchiveFormat, EntryKind};
use crate::hash::{ContentHasher, HashKind};
use crate::sync::scanner::ScanResult;

/// RN-50: `<tarefa>_AAAA-MM-DD_HH-MM-SS.<ext>`, na hora local. Caracteres que não podem
/// ir num nome de arquivo viram `_`. Se o nome já estiver em uso no destino (`taken`),
/// acrescenta ` (2)`, ` (3)`… — nunca sobrescreve.
pub fn version_name(
    job_name: &str,
    at: NaiveDateTime,
    format: ArchiveFormat,
    taken: impl Fn(&str) -> bool,
) -> String {
    let clean: String = job_name
        .trim()
        .chars()
        .map(|c| {
            if c == '/' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Sem ponto no início (arquivo oculto) nem nomes só de pontos.
    let clean = clean.trim_start_matches('.');
    let base = if clean.is_empty() { "tarefa" } else { clean };
    let stamp = at.format("%Y-%m-%d_%H-%M-%S");
    let ext = format.extension();
    let mut stem = format!("{base}_{stamp}");
    if validate_name(&format!("{stem}.{ext}")).is_err() {
        stem = format!("tarefa_{stamp}");
    }
    let mut name = format!("{stem}.{ext}");
    let mut n = 2;
    while taken(&name) {
        name = format!("{stem} ({n}).{ext}");
        n += 1;
    }
    name
}

/// RN-59: resumo do que seria compactado. Muda se algum caminho for criado/removido ou se
/// tamanho/data de algum arquivo mudar. A data das pastas não entra (mudaria à toa).
pub fn fingerprint(scan: &ScanResult) -> String {
    let mut hasher = ContentHasher::new(HashKind::Md5);
    for (key, entry) in &scan.entries {
        hasher.update(key.as_bytes());
        hasher.update(&[0]);
        match entry.kind {
            EntryKind::Folder => hasher.update(b"d\n"),
            EntryKind::File => {
                hasher.update(b"f");
                hasher.update(&entry.size.to_le_bytes());
                hasher.update(&entry.mtime_ns.to_le_bytes());
                hasher.update(b"\n");
            }
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::LocalEntry;

    fn at() -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 14)
            .unwrap()
            .and_hms_opt(2, 0, 5)
            .unwrap()
    }

    #[test]
    fn rn50_nome_da_versao() {
        let free = |_: &str| false;
        assert_eq!(
            version_name("Mundo", at(), ArchiveFormat::TarZst, free),
            "Mundo_2026-10-14_02-00-05.tar.zst"
        );
        assert_eq!(
            version_name("a/b\\c", at(), ArchiveFormat::Zip, free),
            "a_b_c_2026-10-14_02-00-05.zip"
        );
        assert_eq!(
            version_name("..", at(), ArchiveFormat::Tar, free),
            "tarefa_2026-10-14_02-00-05.tar"
        );
        // Nunca sobrescreve: nomes em uso ganham sufixo.
        let used = [
            "Mundo_2026-10-14_02-00-05.zip",
            "Mundo_2026-10-14_02-00-05 (2).zip",
        ];
        assert_eq!(
            version_name("Mundo", at(), ArchiveFormat::Zip, |n| used.contains(&n)),
            "Mundo_2026-10-14_02-00-05 (3).zip"
        );
    }

    fn scan_with(size: u64, mtime: i64) -> ScanResult {
        let mut scan = ScanResult::default();
        scan.entries.insert(
            "Mundo".into(),
            LocalEntry {
                kind: EntryKind::Folder,
                size: 0,
                mtime_ns: mtime,
                hash: None,
            },
        );
        scan.entries.insert(
            "Mundo/level.dat".into(),
            LocalEntry {
                kind: EntryKind::File,
                size,
                mtime_ns: mtime,
                hash: None,
            },
        );
        scan
    }

    #[test]
    fn rn59_impressao_digital_so_muda_com_o_conteudo() {
        let base = fingerprint(&scan_with(10, 1));
        assert_eq!(base, fingerprint(&scan_with(10, 1)));
        assert_ne!(base, fingerprint(&scan_with(11, 1)));
        assert_ne!(base, fingerprint(&scan_with(10, 2)));
        let mut extra = scan_with(10, 1);
        extra.entries.insert(
            "Mundo/novo.txt".into(),
            LocalEntry {
                kind: EntryKind::File,
                size: 1,
                mtime_ns: 1,
                hash: None,
            },
        );
        assert_ne!(base, fingerprint(&extra));
    }
}
