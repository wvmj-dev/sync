//! Compactar (RN-60 a RN-62, RN-64): a lista de entradas vem do scanner, então valem os
//! mesmos filtros da sincronização (lixo, arquivos sensíveis, `.syncignore`, links).

use std::fs::{self, File, Metadata};
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use chrono::{Datelike, Timelike};
use serde::Serialize;
use zip::write::SimpleFileOptions;

use super::{ArchiveFormat, ArchiveProgress, CANCELLED, Tracked, temp_sibling};

/// Entrega exatamente `size` bytes: o cabeçalho do TAR declara o tamanho antes dos dados,
/// e a biblioteca não confere — um arquivo que encolhesse geraria um TAR corrompido.
struct Exact<R> {
    inner: io::Take<R>,
    remaining: u64,
}

impl<R: Read> Exact<R> {
    fn new(inner: R, size: u64) -> Self {
        Self {
            inner: inner.take(size),
            remaining: size,
        }
    }
}

impl<R: Read> Read for Exact<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n == 0 && self.remaining > 0 && !buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "o arquivo mudou durante a compactação — tente de novo",
            ));
        }
        self.remaining -= n as u64;
        Ok(n)
    }
}
use crate::domain::EntryKind;
use crate::error::{CoreError, CoreResult, IoContext};
use crate::sync::scanner::ScanResult;

/// RN-61: já comprimidos — no ZIP vão sem recompressão (seria só tempo perdido).
const ALREADY_COMPRESSED: &[&str] = &[
    "jpg", "jpeg", "png", "gif", "webp", "heic", "avif", "mp3", "mp4", "m4a", "m4v", "mkv", "mov",
    "avi", "webm", "ogg", "opus", "flac", "zip", "gz", "tgz", "bz2", "xz", "zst", "7z", "rar",
    "jar", "apk", "docx", "xlsx", "pptx", "odt", "ods", "odp", "epub", "woff2",
];

pub struct CreateRequest<'a> {
    /// Entradas a incluir (chaves `Fonte/caminho`), já filtradas.
    pub scan: &'a ScanResult,
    /// Caminho final; não pode existir (RN-62).
    pub output: &'a Path,
    pub format: ArchiveFormat,
    /// Nível de compressão; `None` = padrão do formato (RN-61).
    pub level: Option<i32>,
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(&ArchiveProgress) + Send + Sync),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateReport {
    pub path: PathBuf,
    pub files: u64,
    pub folders: u64,
    pub bytes_in: u64,
    pub bytes_out: u64,
}

pub fn create(req: &CreateRequest<'_>) -> CoreResult<CreateReport> {
    if req.output.exists() {
        return Err(already_exists(req.output));
    }
    let tmp = temp_sibling(req.output);
    let result = Writer::new(req).write(&tmp);
    let mut report = match result {
        Ok(report) => report,
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    };
    publish(&tmp, req.output)?;
    report.bytes_out = fs::metadata(req.output).map_or(0, |m| m.len());
    report.path = req.output.to_path_buf();
    Ok(report)
}

/// RN-62: publica o temporário sem nunca sobrescrever. `hard_link` falha se o destino
/// existir — sem janela de corrida entre conferir e renomear.
fn publish(tmp: &Path, output: &Path) -> CoreResult<()> {
    let result = match fs::hard_link(tmp, output) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(already_exists(output)),
        // Discos sem hard link (ex.: FAT/exFAT): confere e renomeia.
        Err(_) if !output.exists() => fs::rename(tmp, output).at(output),
        Err(e) => Err(e).at(output),
    };
    let _ = fs::remove_file(tmp);
    result
}

fn already_exists(path: &Path) -> CoreError {
    CoreError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "já existe — nada foi sobrescrito",
        ),
    }
}

struct Writer<'a> {
    req: &'a CreateRequest<'a>,
    report: CreateReport,
    progress: ArchiveProgress,
}

impl<'a> Writer<'a> {
    fn new(req: &'a CreateRequest<'a>) -> Self {
        let files = req
            .scan
            .entries
            .values()
            .filter(|e| e.kind == EntryKind::File);
        let progress = ArchiveProgress {
            files_total: files.clone().count() as u64,
            bytes_total: files.map(|e| e.size).sum(),
            ..ArchiveProgress::default()
        };
        Self {
            req,
            report: CreateReport::default(),
            progress,
        }
    }

    fn write(mut self, tmp: &Path) -> CoreResult<CreateReport> {
        let file = File::options()
            .write(true)
            .create_new(true)
            .open(tmp)
            .at(tmp)?;
        (self.req.progress)(&self.progress);
        let out = BufWriter::new(file);
        let out = match self.req.format {
            ArchiveFormat::Zip => self.zip(out)?,
            ArchiveFormat::Tar => self.tar(out)?,
            ArchiveFormat::TarGz => {
                let level = self.req.level.map_or(6, |l| l.clamp(0, 9)) as u32;
                let encoder = flate2::write::GzEncoder::new(out, flate2::Compression::new(level));
                self.tar(encoder)?.finish().at(tmp)?
            }
            ArchiveFormat::TarZst => {
                let level = self.req.level.map_or(3, |l| l.clamp(1, 19));
                let mut encoder = zstd::Encoder::new(out, level).at(tmp)?;
                let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
                encoder.multithread(threads as u32).at(tmp)?;
                self.tar(encoder)?.finish().at(tmp)?
            }
        };
        let file = out.into_inner().map_err(|e| e.into_error()).at(tmp)?;
        file.sync_all().at(tmp)?;
        Ok(self.report)
    }

    /// Itera as entradas em ordem (pasta antes do conteúdo), parando se cancelado.
    fn each(
        &mut self,
        mut on: impl FnMut(&mut Self, &str, EntryKind, i64) -> CoreResult<()>,
    ) -> CoreResult<()> {
        let entries: Vec<_> = self
            .req
            .scan
            .entries
            .iter()
            .map(|(k, e)| (k.clone(), e.kind, e.mtime_ns))
            .collect();
        for (key, kind, mtime) in entries {
            if self.req.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err(CoreError::Cancelled);
            }
            on(self, &key, kind, mtime)?;
        }
        Ok(())
    }

    fn open(&mut self, key: &str) -> CoreResult<(File, Metadata, PathBuf)> {
        let abs = self
            .req
            .scan
            .local_path(key)
            .ok_or_else(|| CoreError::Config(format!("caminho local desconhecido: {key}")))?;
        let file = File::open(&abs).at(&abs)?;
        let meta = file.metadata().at(&abs)?;
        self.progress.current = Some(key.to_string());
        (self.req.progress)(&self.progress);
        Ok((file, meta, abs))
    }

    fn file_done(&mut self, size: u64) {
        self.report.files += 1;
        self.report.bytes_in += size;
        self.progress.files_done += 1;
        (self.req.progress)(&self.progress);
    }

    fn tar<W: Write>(&mut self, out: W) -> CoreResult<W> {
        let mut builder = tar::Builder::new(out);
        let output = self.req.output.to_path_buf();
        self.each(|w, key, kind, mtime| {
            let mut header = tar::Header::new_gnu();
            header.set_mtime(u64::try_from(mtime / 1_000_000_000).unwrap_or(0));
            match kind {
                EntryKind::Folder => {
                    header.set_entry_type(tar::EntryType::Directory);
                    header.set_size(0);
                    header.set_mode(0o755);
                    builder
                        .append_data(&mut header, format!("{key}/"), io::empty())
                        .at(&output)?;
                    w.report.folders += 1;
                }
                EntryKind::File => {
                    let (file, meta, abs) = w.open(key)?;
                    let size = meta.len();
                    header.set_entry_type(tar::EntryType::Regular);
                    header.set_size(size);
                    header.set_mode(file_mode(&meta));
                    let reader = Tracked::new(
                        Exact::new(file, size),
                        w.req.cancel,
                        &mut w.progress,
                        w.req.progress,
                    );
                    builder
                        .append_data(&mut header, key, reader)
                        .map_err(|e| io_error(e, &abs))?;
                    w.file_done(size);
                }
            }
            Ok(())
        })?;
        builder.into_inner().at(&output)
    }

    fn zip<W: Write + io::Seek>(&mut self, out: W) -> CoreResult<W> {
        let output = self.req.output.to_path_buf();
        let mut zip = zip::ZipWriter::new(out);
        let level = self.req.level.map(|l| i64::from(l.clamp(0, 9)));
        self.each(|w, key, kind, mtime| {
            let base = SimpleFileOptions::default().last_modified_time(zip_time(mtime));
            match kind {
                EntryKind::Folder => {
                    zip.add_directory(format!("{key}/"), base.unix_permissions(0o755))
                        .map_err(|e| zip_error(e, &output))?;
                    w.report.folders += 1;
                }
                EntryKind::File => {
                    let (file, meta, abs) = w.open(key)?;
                    let size = meta.len();
                    let stored = already_compressed(key);
                    let options = base
                        .compression_method(if stored {
                            zip::CompressionMethod::Stored
                        } else {
                            zip::CompressionMethod::Deflated
                        })
                        .compression_level(if stored { None } else { level })
                        .unix_permissions(file_mode(&meta))
                        .large_file(size >= u64::from(u32::MAX));
                    zip.start_file(key, options)
                        .map_err(|e| zip_error(e, &abs))?;
                    let mut reader =
                        Tracked::new(file, w.req.cancel, &mut w.progress, w.req.progress);
                    io::copy(&mut reader, &mut zip).map_err(|e| io_error(e, &abs))?;
                    w.file_done(size);
                }
            }
            Ok(())
        })?;
        zip.finish().map_err(|e| zip_error(e, &output))
    }
}

fn already_compressed(key: &str) -> bool {
    key.rsplit_once('.')
        .is_some_and(|(_, ext)| ALREADY_COMPRESSED.contains(&ext.to_ascii_lowercase().as_str()))
}

/// Só o bit de execução é preservado; setuid/setgid nunca (RN-66).
fn file_mode(meta: &Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 != 0 {
            return 0o755;
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    0o644
}

fn zip_time(mtime_ns: i64) -> zip::DateTime {
    let Some(t) = chrono::DateTime::from_timestamp(mtime_ns.div_euclid(1_000_000_000), 0) else {
        return zip::DateTime::default();
    };
    let t = t.with_timezone(&chrono::Local);
    let fit = |v: u32| u8::try_from(v).unwrap_or(0);
    zip::DateTime::from_date_and_time(
        u16::try_from(t.year()).unwrap_or(1980),
        fit(t.month()),
        fit(t.day()),
        fit(t.hour()),
        fit(t.minute()),
        fit(t.second()),
    )
    .unwrap_or_default()
}

fn io_error(e: io::Error, path: &Path) -> CoreError {
    if e.kind() == io::ErrorKind::Interrupted && e.to_string() == CANCELLED {
        return CoreError::Cancelled;
    }
    CoreError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

fn zip_error(e: zip::result::ZipError, path: &Path) -> CoreError {
    match e {
        zip::result::ZipError::Io(io) => io_error(io, path),
        other => CoreError::Io {
            path: path.to_path_buf(),
            source: io::Error::other(other.to_string()),
        },
    }
}
