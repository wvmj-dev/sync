//! Descompactar (RN-65 a RN-67). Arquivos compactados vindos de fora são dados não
//! confiáveis: cada entrada é validada antes de tocar o disco.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::{ArchiveFormat, ArchiveProgress, CANCELLED, safe_entry_path, temp_sibling};
use crate::error::{CoreError, CoreResult, IoContext, ValidationIssue};

const BUF_SIZE: usize = 256 * 1024;
const PROGRESS_STEP: u64 = 4 * 1024 * 1024;

pub struct ExtractRequest<'a> {
    pub archive: &'a Path,
    /// Pasta final; não pode existir (RN-65).
    pub dest: &'a Path,
    /// RN-67: espaço que precisa continuar livre no disco.
    pub reserve_bytes: u64,
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(&ArchiveProgress) + Send + Sync),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkippedEntry {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractReport {
    pub dest: PathBuf,
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    pub skipped: Vec<SkippedEntry>,
}

pub fn extract(req: &ExtractRequest<'_>) -> CoreResult<ExtractReport> {
    let file_name = req
        .archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let Some((format, _)) = ArchiveFormat::detect(file_name) else {
        return Err(CoreError::Invalid(vec![ValidationIssue::new(
            "arquivo",
            "formato não suportado — use .zip, .tar, .tar.gz ou .tar.zst",
        )]));
    };
    if req.dest.exists() {
        return Err(CoreError::Io {
            path: req.dest.to_path_buf(),
            source: io::Error::new(
                io::ErrorKind::AlreadyExists,
                "já existe — nada foi sobrescrito",
            ),
        });
    }

    // RN-65: tudo vai para uma pasta temporária ao lado do destino; só ganha o nome final
    // se a extração terminar bem. Erro ou cancelamento não deixam nada pela metade.
    let tmp = temp_sibling(req.dest);
    fs::create_dir(&tmp).at(&tmp)?;
    let available = fs4::available_space(&tmp).unwrap_or(u64::MAX);
    let mut sink = Sink {
        root: &tmp,
        req,
        budget: available.saturating_sub(req.reserve_bytes),
        report: ExtractReport::default(),
        progress: ArchiveProgress::default(),
        last_notify: 0,
    };
    let result = File::open(req.archive)
        .at(req.archive)
        .and_then(|file| match format {
            ArchiveFormat::Zip => sink.zip(file),
            ArchiveFormat::Tar => sink.tar(BufReader::new(file)),
            ArchiveFormat::TarGz => sink.tar(flate2::read::GzDecoder::new(BufReader::new(file))),
            ArchiveFormat::TarZst => {
                let decoder = zstd::Decoder::new(file).at(req.archive)?;
                sink.tar(decoder)
            }
        });
    let mut report = sink.report;
    let published = result.and_then(|()| {
        if req.dest.exists() {
            return Err(CoreError::Io {
                path: req.dest.to_path_buf(),
                source: io::Error::new(io::ErrorKind::AlreadyExists, "já existe"),
            });
        }
        // Uma única pasta no topo (ex.: `Mundo/`) vira a própria pasta de destino —
        // evita `Mundo/Mundo/...`.
        let only_child = single_dir(&tmp);
        match only_child {
            Some(inner) => {
                fs::rename(&inner, req.dest).at(req.dest)?;
                fs::remove_dir(&tmp).at(&tmp)
            }
            None => fs::rename(&tmp, req.dest).at(req.dest),
        }
    });
    if let Err(e) = published {
        // Pasta temporária do próprio app — nada do usuário é apagado.
        let _ = fs::remove_dir_all(&tmp);
        return Err(e);
    }
    report.dest = req.dest.to_path_buf();
    Ok(report)
}

struct Sink<'a> {
    root: &'a Path,
    req: &'a ExtractRequest<'a>,
    /// RN-67: bytes que ainda podem ser escritos.
    budget: u64,
    report: ExtractReport,
    progress: ArchiveProgress,
    last_notify: u64,
}

impl Sink<'_> {
    fn check_cancel(&self) -> CoreResult<()> {
        if self.req.cancel.load(Ordering::Relaxed) {
            return Err(CoreError::Cancelled);
        }
        Ok(())
    }

    fn skip(&mut self, name: &str, reason: &str) {
        self.report.skipped.push(SkippedEntry {
            name: name.to_string(),
            reason: reason.to_string(),
        });
    }

    /// Caminho dentro da pasta temporária, ou `None` (e registro) se a entrada for perigosa.
    fn target(&mut self, name: &str) -> Option<PathBuf> {
        match safe_entry_path(name) {
            Some(parts) => Some(parts.iter().fold(self.root.to_path_buf(), |p, c| p.join(c))),
            None => {
                self.skip(
                    name,
                    "caminho inseguro (absoluto, com '..' ou nome inválido)",
                );
                None
            }
        }
    }

    fn dir(&mut self, name: &str) -> CoreResult<()> {
        let Some(path) = self.target(name) else {
            return Ok(());
        };
        match fs::create_dir_all(&path) {
            Ok(()) => self.report.folders += 1,
            Err(_) => self.skip(name, "conflita com um arquivo de mesmo nome"),
        }
        Ok(())
    }

    fn file(
        &mut self,
        name: &str,
        mode: Option<u32>,
        mtime: Option<SystemTime>,
        reader: &mut dyn Read,
    ) -> CoreResult<()> {
        let Some(path) = self.target(name) else {
            return Ok(());
        };
        if let Some(parent) = path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            self.skip(name, "conflita com um arquivo de mesmo nome");
            return Ok(());
        }
        // Nunca sobrescreve — nem uma entrada repetida dentro do próprio arquivo.
        let mut out = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                self.skip(name, "entrada repetida no arquivo compactado");
                return Ok(());
            }
            Err(_) => {
                self.skip(name, "conflita com um arquivo de mesmo nome");
                return Ok(());
            }
        };
        self.progress.current = Some(name.to_string());
        (self.req.progress)(&self.progress);

        let mut buf = vec![0u8; BUF_SIZE];
        loop {
            self.check_cancel()?;
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted && e.to_string() != CANCELLED => {
                    continue;
                }
                Err(e) => return Err(e).at(self.req.archive),
            };
            // RN-67: proteção contra "bomba de compressão" (arquivo minúsculo que vira terabytes).
            if (n as u64) > self.budget {
                return Err(CoreError::DiskFull(
                    "descompactar ocuparia o espaço livre do disco — operação interrompida".into(),
                ));
            }
            self.budget -= n as u64;
            out.write_all(&buf[..n]).at(&path)?;
            self.report.bytes += n as u64;
            self.progress.bytes_done += n as u64;
            if self.progress.bytes_done - self.last_notify >= PROGRESS_STEP {
                self.last_notify = self.progress.bytes_done;
                (self.req.progress)(&self.progress);
            }
        }
        // RN-66: só o bit de execução é mantido; setuid/setgid nunca.
        #[cfg(unix)]
        if mode.is_some_and(|m| m & 0o111 != 0) {
            use std::os::unix::fs::PermissionsExt;
            let _ = out.set_permissions(fs::Permissions::from_mode(0o755));
        }
        #[cfg(not(unix))]
        let _ = mode;
        if let Some(t) = mtime {
            let _ = out.set_modified(t);
        }
        self.report.files += 1;
        self.progress.files_done += 1;
        Ok(())
    }

    fn zip(&mut self, file: File) -> CoreResult<()> {
        let archive_path = self.req.archive;
        let zip_err = |e: zip::result::ZipError| CoreError::Io {
            path: archive_path.to_path_buf(),
            source: io::Error::other(e.to_string()),
        };
        let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(zip_err)?;
        self.progress.files_total = zip.len() as u64;
        for i in 0..zip.len() {
            self.check_cancel()?;
            let mut entry = zip.by_index(i).map_err(zip_err)?;
            let name = entry.name().map_err(zip_err)?.to_string();
            if entry.is_symlink() {
                self.skip(&name, "links não são extraídos");
            } else if entry.is_dir() {
                self.dir(&name)?;
            } else {
                let mode = entry.unix_mode();
                let mtime = entry.last_modified().and_then(zip_time);
                self.file(&name, mode, mtime, &mut entry)?;
            }
        }
        Ok(())
    }

    fn tar(&mut self, reader: impl Read) -> CoreResult<()> {
        let archive_path = self.req.archive;
        let mut archive = tar::Archive::new(reader);
        for entry in archive.entries().at(archive_path)? {
            self.check_cancel()?;
            let mut entry = entry.at(archive_path)?;
            let name = match std::str::from_utf8(&entry.path_bytes()) {
                Ok(n) => n.to_string(),
                Err(_) => {
                    self.skip("(nome não UTF-8)", "nome inválido");
                    continue;
                }
            };
            let header = entry.header();
            let kind = header.entry_type();
            let mode = header.mode().ok();
            let mtime = header
                .mtime()
                .ok()
                .map(|s| UNIX_EPOCH + Duration::from_secs(s));
            match kind {
                tar::EntryType::Directory => self.dir(&name)?,
                tar::EntryType::Regular | tar::EntryType::Continuous => {
                    self.file(&name, mode, mtime, &mut entry)?;
                }
                tar::EntryType::XGlobalHeader | tar::EntryType::XHeader => {}
                _ => self.skip(&name, "links e arquivos especiais não são extraídos"),
            }
        }
        Ok(())
    }
}

/// A pasta tem exatamente um item, e ele é uma pasta?
fn single_dir(dir: &Path) -> Option<PathBuf> {
    let mut items = fs::read_dir(dir).ok()?;
    let first = items.next()?.ok()?;
    if items.next().is_some() || !first.file_type().ok()?.is_dir() {
        return None;
    }
    Some(first.path())
}

fn zip_time(t: zip::DateTime) -> Option<SystemTime> {
    let date = chrono::NaiveDate::from_ymd_opt(
        i32::from(t.year()),
        u32::from(t.month()),
        u32::from(t.day()),
    )?;
    let local = date
        .and_hms_opt(
            u32::from(t.hour()),
            u32::from(t.minute()),
            u32::from(t.second()),
        )?
        .and_local_timezone(chrono::Local)
        .single()?;
    Some(local.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::{CreateRequest, create};
    use crate::filters::Filter;
    use crate::sync::scanner::{self, ScanRequest};
    use std::collections::BTreeMap;

    fn source() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Mundo");
        fs::create_dir_all(src.join("region")).unwrap();
        fs::create_dir_all(src.join("vazia")).unwrap();
        fs::write(src.join("level.dat"), "nível").unwrap();
        fs::write(src.join("region/r.0.mca"), vec![7u8; 300_000]).unwrap();
        fs::write(src.join("foto.jpg"), vec![1u8; 1000]).unwrap();
        fs::write(src.join(".env"), "SEGREDO=1").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::write(src.join("rodar.sh"), "#!/bin/sh").unwrap();
            fs::set_permissions(src.join("rodar.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        }
        (tmp, src)
    }

    fn scan(src: &Path) -> scanner::ScanResult {
        scanner::scan(
            &ScanRequest {
                sources: vec![src.to_path_buf()],
                filter: Filter::new(&[], false).unwrap(),
                skip_dirs: vec![],
                hash_kind: None,
            },
            &BTreeMap::new(),
        )
    }

    fn compress(src: &Path, out: &Path, format: ArchiveFormat) -> crate::archive::CreateReport {
        let scanned = scan(src);
        create(&CreateRequest {
            scan: &scanned,
            output: out,
            format,
            level: None,
            cancel: &AtomicBool::new(false),
            progress: &|_| {},
        })
        .unwrap()
    }

    fn unpack(
        archive: &Path,
        dest: &Path,
        reserve: u64,
        cancel: bool,
    ) -> CoreResult<ExtractReport> {
        extract(&ExtractRequest {
            archive,
            dest,
            reserve_bytes: reserve,
            cancel: &AtomicBool::new(cancel),
            progress: &|_| {},
        })
    }

    #[test]
    fn ida_e_volta_em_todos_os_formatos() {
        let (tmp, src) = source();
        for format in ArchiveFormat::ALL {
            let out = tmp.path().join(format!("mundo.{}", format.extension()));
            let created = compress(&src, &out, format);
            assert_eq!(created.files, if cfg!(unix) { 4 } else { 3 }, "{format:?}");
            assert!(created.bytes_out > 0);

            let dest = tmp.path().join(format!("extraido-{}", format.extension()));
            let report = unpack(&out, &dest, 0, false).unwrap();
            assert!(
                report.skipped.is_empty(),
                "{format:?}: {:?}",
                report.skipped
            );
            // Pasta única no topo: o conteúdo vai direto para o destino (sem Mundo/Mundo).
            assert_eq!(fs::read_to_string(dest.join("level.dat")).unwrap(), "nível");
            assert_eq!(
                fs::read(dest.join("region/r.0.mca")).unwrap().len(),
                300_000
            );
            assert!(dest.join("vazia").is_dir());
            // RN-64: arquivo sensível não entra no compactado.
            assert!(!dest.join(".env").exists());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = fs::metadata(dest.join("rodar.sh"))
                    .unwrap()
                    .permissions()
                    .mode();
                assert_eq!(mode & 0o111, 0o111, "{format:?}: bit de execução");
            }
        }
    }

    #[test]
    fn rn62_nunca_sobrescreve_o_arquivo_de_saida() {
        let (tmp, src) = source();
        let out = tmp.path().join("x.zip");
        fs::write(&out, "meu arquivo").unwrap();
        let scanned = scan(&src);
        let err = create(&CreateRequest {
            scan: &scanned,
            output: &out,
            format: ArchiveFormat::Zip,
            level: None,
            cancel: &AtomicBool::new(false),
            progress: &|_| {},
        })
        .unwrap_err();
        assert!(err.to_string().contains("já existe"));
        assert_eq!(fs::read_to_string(&out).unwrap(), "meu arquivo");
    }

    #[test]
    fn rn66_zip_malicioso_nao_escreve_fora_do_destino() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("ruim.zip");
        {
            let mut zip = zip::ZipWriter::new(File::create(&archive).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            for name in ["../fora.txt", "/abs.txt", "a/../../fora2.txt", "ok/bom.txt"] {
                zip.start_file(name, opts).unwrap();
                zip.write_all(b"x").unwrap();
            }
            zip.finish().unwrap();
        }
        let dest = tmp.path().join("dest").join("saida");
        fs::create_dir_all(dest.parent().unwrap()).unwrap();
        let report = unpack(&archive, &dest, 0, false).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped.len(), 3);
        assert!(dest.join("bom.txt").exists());
        assert!(!tmp.path().join("fora.txt").exists());
        assert!(!tmp.path().join("dest/fora.txt").exists());
        assert!(!tmp.path().join("fora2.txt").exists());
    }

    /// TAR montado à mão, com nomes que a biblioteca normalmente recusaria.
    fn raw_tar(path: &Path, entries: &[(&str, tar::EntryType, &[u8])]) {
        let mut builder = tar::Builder::new(File::create(path).unwrap());
        for (name, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            let gnu = header.as_gnu_mut().unwrap();
            gnu.name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o4755); // setuid: nunca deve ser aplicado
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.finish().unwrap();
    }

    #[test]
    fn rn66_tar_malicioso_links_e_setuid() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("ruim.tar");
        raw_tar(
            &archive,
            &[
                ("../fora.txt", tar::EntryType::Regular, b"x"),
                ("link", tar::EntryType::Symlink, b""),
                ("ok.sh", tar::EntryType::Regular, b"#!/bin/sh"),
            ],
        );
        let dest = tmp.path().join("saida");
        let report = unpack(&archive, &dest, 0, false).unwrap();
        assert_eq!(report.files, 1);
        assert_eq!(report.skipped.len(), 2);
        assert!(!tmp.path().join("fora.txt").exists());
        assert!(fs::symlink_metadata(dest.join("link")).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dest.join("ok.sh"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o7000, 0, "setuid/setgid nunca são aplicados");
        }
    }

    #[test]
    fn rn67_bomba_de_compressao_e_interrompida_sem_deixar_lixo() {
        let (tmp, src) = source();
        let out = tmp.path().join("m.tar.zst");
        compress(&src, &out, ArchiveFormat::TarZst);
        let dest = tmp.path().join("saida");
        let err = unpack(&out, &dest, u64::MAX, false).unwrap_err();
        assert!(matches!(err, CoreError::DiskFull(_)));
        assert!(!dest.exists());
        let leftovers = fs::read_dir(tmp.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".syncly-tmp")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn cancelar_nao_deixa_nada_pela_metade() {
        let (tmp, src) = source();
        let out = tmp.path().join("m.zip");
        compress(&src, &out, ArchiveFormat::Zip);
        let dest = tmp.path().join("saida");
        assert!(matches!(
            unpack(&out, &dest, 0, true),
            Err(CoreError::Cancelled)
        ));
        assert!(!dest.exists());
    }

    #[test]
    fn formato_desconhecido_e_recusado() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("x.rar");
        fs::write(&archive, "x").unwrap();
        assert!(matches!(
            unpack(&archive, &tmp.path().join("d"), 0, false),
            Err(CoreError::Invalid(_))
        ));
    }
}
