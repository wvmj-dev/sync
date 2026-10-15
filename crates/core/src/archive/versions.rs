//! Versões de tarefas compactadas (RN-50, RN-59, RN-107): nome com data, "impressão
//! digital" do conteúdo das fontes e escolha da versão mais recente ao receber.

use chrono::NaiveDateTime;

use crate::domain::rel_path::validate_name;
use crate::domain::{ArchiveFormat, EntryKind};
use crate::hash::{ContentHasher, HashKind};
use crate::provider::RemoteEntry;
use crate::sync::scanner::ScanResult;

/// Data e sequência lidas do nome de uma versão (`… (2).zip` → 2; sem sufixo → 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct VersionStamp {
    pub at: NaiveDateTime,
    pub seq: u32,
}

/// RN-107: reconhece `<tarefa>_AAAA-MM-DD_HH-MM-SS[ (n)].<ext>` (RN-50). Qualquer outro
/// nome não é uma versão — fica de fora do recebimento.
pub fn parse_version_name(name: &str) -> Option<(VersionStamp, ArchiveFormat)> {
    let (format, stem) = ArchiveFormat::detect(name)?;
    let (stem, seq) = match stem.strip_suffix(')').and_then(|s| s.rsplit_once(" (")) {
        Some((base, n)) => (base, n.parse::<u32>().ok().filter(|n| *n >= 2)?),
        None => (stem, 1),
    };
    // "_" + "AAAA-MM-DD_HH-MM-SS" (19 caracteres ASCII), com algo antes.
    let split = stem.len().checked_sub(20).filter(|i| *i > 0)?;
    let (base, stamp) = stem.split_at_checked(split)?;
    let stamp = stamp.strip_prefix('_')?;
    if base.is_empty() {
        return None;
    }
    let at = NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d_%H-%M-%S").ok()?;
    Some((VersionStamp { at, seq }, format))
}

/// RN-107: a versão mais recente entre os arquivos de uma pasta, pela data no nome.
/// Nomes repetidos são devolvidos à parte: o app nunca escolhe entre eles (RN-34).
pub fn latest_version(entries: &[RemoteEntry]) -> Result<Option<&RemoteEntry>, String> {
    let best = entries
        .iter()
        .filter(|e| e.kind == EntryKind::File)
        .filter_map(|e| parse_version_name(&e.name).map(|(stamp, _)| (stamp, e)))
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)))
        .map(|(_, e)| e);
    match best {
        Some(e) if entries.iter().filter(|o| o.name == e.name).count() > 1 => Err(format!(
            "há mais de um item '{}' no destino — renomeie no provedor",
            e.name
        )),
        other => Ok(other),
    }
}

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

    fn file(name: &str) -> RemoteEntry {
        RemoteEntry {
            id: format!("id-{name}"),
            name: name.into(),
            kind: EntryKind::File,
            size: Some(1),
            hash: Some("h".into()),
            modified_at: None,
            modified_by: None,
        }
    }

    #[test]
    fn rn107_reconhece_nomes_de_versao() {
        let (stamp, format) = parse_version_name("Mundo_2026-10-14_02-00-05.tar.zst").unwrap();
        assert_eq!(
            (stamp.at, stamp.seq, format),
            (at(), 1, ArchiveFormat::TarZst)
        );
        let (stamp, _) = parse_version_name("Meu mundo_2026-10-14_02-00-05 (3).zip").unwrap();
        assert_eq!(stamp.seq, 3);
        // O que o próprio app gera é sempre reconhecido.
        let name = version_name("Mundo", at(), ArchiveFormat::TarGz, |_| false);
        assert!(parse_version_name(&name).is_some());
        for other in [
            "Mundo.zip",
            "_2026-10-14_02-00-05.zip",
            "Mundo_2026-13-14_02-00-05.zip",
            "Mundo_2026-10-14_02-00-05.rar",
            "Mundo_2026-10-14_02-00-05 (x).zip",
            "Mundo_2026-10-14_02-00-05 (1).zip",
            "notas.txt",
        ] {
            assert!(parse_version_name(other).is_none(), "{other}");
        }
    }

    #[test]
    fn rn107_escolhe_a_versao_mais_recente() {
        let entries = [
            file("Mundo_2026-10-13_23-59-59.zip"),
            file("Mundo_2026-10-14_02-00-05.zip"),
            file("Mundo_2026-10-14_02-00-05 (2).zip"),
            file("Mundo_2027-01-01_00-00-00.txt"),
            file("leia-me.zip"),
        ];
        let latest = latest_version(&entries).unwrap().unwrap();
        assert_eq!(latest.name, "Mundo_2026-10-14_02-00-05 (2).zip");
        assert_eq!(latest_version(&entries[3..]).unwrap(), None);
        // RN-34: nome repetido — o app não escolhe.
        let dup = [
            file("Mundo_2026-10-14_02-00-05.zip"),
            file("Mundo_2026-10-14_02-00-05.zip"),
        ];
        assert!(latest_version(&dup).is_err());
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
