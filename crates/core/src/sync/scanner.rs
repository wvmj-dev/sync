//! Varre as fontes locais aplicando filtros (RN-20 a RN-28).
//!
//! Velocidade: só calcula hash de arquivos cujo tamanho ou mtime mudou desde o
//! índice (RN-26), e calcula em paralelo.
//!
//! Segurança: o que não pôde ser lido vai para `blocked` (RN-27) e o que os
//! filtros excluíram vai para `excluded` (RN-28). O planejador nunca manda para a
//! lixeira nada abaixo desses caminhos — nem um erro de permissão nem uma
//! exclusão nova viram "arquivo apagado".

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::UNIX_EPOCH;

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;

use crate::domain::rel_path::{is_within_str, join_str, validate_name};
use crate::domain::{EntryKind, IndexEntry, Job, LocalEntry};
use crate::filters::{Filter, IGNORE_FILE_NAME};
use crate::hash::{HashKind, hash_file};

#[derive(Debug, Clone)]
pub struct ScanRequest {
    pub sources: Vec<PathBuf>,
    pub filter: Filter,
    /// Pastas do app, puladas mesmo se estiverem dentro de uma fonte (RN-12).
    pub skip_dirs: Vec<PathBuf>,
    pub hash_kind: HashKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanWarning {
    /// RN-24
    Symlink(PathBuf),
    Special(PathBuf),
    /// Nome não-UTF-8 ou com caracteres de controle.
    InvalidName(PathBuf),
    /// A própria fonte casa com uma exclusão (RN-22).
    ExcludedSource(PathBuf),
    Unreadable {
        path: PathBuf,
        error: String,
    },
}

impl ScanWarning {
    pub fn describe(&self) -> String {
        match self {
            Self::Symlink(p) => format!("link simbólico ignorado: {}", p.display()),
            Self::Special(p) => format!("arquivo especial ignorado: {}", p.display()),
            Self::InvalidName(p) => format!("nome inválido ignorado: {}", p.display()),
            Self::ExcludedSource(p) => {
                format!("fonte ignorada por casar com uma exclusão: {}", p.display())
            }
            Self::Unreadable { path, error } => {
                format!("não foi possível ler {}: {error}", path.display())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    /// Nome no destino (RN-14).
    pub name: String,
    pub path: PathBuf,
    pub exists: bool,
    pub readable: bool,
    pub files: usize,
}

#[derive(Debug, Default)]
pub struct ScanResult {
    /// Chave: caminho relativo ao destino, começando pelo nome da fonte.
    pub entries: BTreeMap<String, LocalEntry>,
    pub sources: Vec<SourceStatus>,
    /// RN-27: estado desconhecido; nada abaixo deles é decidido.
    pub blocked: BTreeSet<String>,
    /// RN-28: excluídos pelos filtros (só o nível mais alto de cada subárvore).
    pub excluded: BTreeSet<String>,
    pub warnings: Vec<ScanWarning>,
    /// Quantos arquivos precisaram de hash (o resto veio do índice).
    pub hashed_files: usize,
}

/// Pasta a visitar, com os `.syncignore` em vigor (do mais raso ao mais fundo).
struct Dir {
    abs: PathBuf,
    key: String,
    rel_in_source: PathBuf,
    ignores: Vec<Rc<Gitignore>>,
}

struct Walker<'a> {
    req: &'a ScanRequest,
    index: &'a BTreeMap<String, IndexEntry>,
    result: ScanResult,
    pending: Vec<(String, PathBuf)>,
}

pub fn scan(req: &ScanRequest, index: &BTreeMap<String, IndexEntry>) -> ScanResult {
    let mut w = Walker {
        req,
        index,
        result: ScanResult::default(),
        pending: Vec::new(),
    };
    for source in &req.sources {
        w.scan_source(source);
    }
    w.hash_pending();

    let mut result = w.result;
    for status in &mut result.sources {
        status.files = result
            .entries
            .range(status.name.clone()..)
            .take_while(|(k, _)| k.starts_with(&status.name))
            .filter(|(k, e)| e.kind == EntryKind::File && is_within_str(k, &status.name))
            .count();
    }
    result
}

impl Walker<'_> {
    fn scan_source(&mut self, source: &Path) {
        let Some(name) = Job::source_name(source).map(str::to_owned) else {
            self.warn(ScanWarning::InvalidName(source.to_path_buf()));
            return;
        };
        let mut status = SourceStatus {
            name: name.clone(),
            path: source.to_path_buf(),
            exists: false,
            readable: false,
            files: 0,
        };

        // A raiz pode ser um link (ex.: ~/Documentos → /dados/docs): foi escolhida
        // explicitamente pelo usuário, então é resolvida. Dentro dela, links não são seguidos.
        let resolved = fs::canonicalize(source).and_then(|root| Ok((fs::metadata(&root)?, root)));
        let (meta, root) = match resolved {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.result.sources.push(status);
                return;
            }
            Err(e) => {
                status.exists = true;
                self.block(name, source, &e.to_string());
                self.result.sources.push(status);
                return;
            }
        };
        status.exists = true;
        status.readable = true;

        // RN-22: a própria fonte também passa pelos filtros (ex.: escolher `~/.ssh`).
        if self.req.filter.is_excluded(Path::new(&name), meta.is_dir()) {
            self.result.excluded.insert(name);
            self.warn(ScanWarning::ExcludedSource(source.to_path_buf()));
        } else if meta.is_file() {
            self.queue_file(name, &root, &meta);
        } else if meta.is_dir() {
            self.result
                .entries
                .insert(name.clone(), folder_entry(&meta));
            self.walk(root, name);
        } else {
            status.readable = false;
            self.warn(ScanWarning::Special(source.to_path_buf()));
        }
        self.result.sources.push(status);
    }

    fn walk(&mut self, root: PathBuf, name: String) {
        let Ok(root_ignores) = self.load_syncignore(&root, &name, Vec::new()) else {
            return;
        };
        let mut stack = vec![Dir {
            abs: root,
            key: name,
            rel_in_source: PathBuf::new(),
            ignores: root_ignores,
        }];

        while let Some(dir) = stack.pop() {
            let items = match fs::read_dir(&dir.abs) {
                Ok(items) => items,
                Err(e) => {
                    self.block(dir.key.clone(), &dir.abs, &e.to_string());
                    continue;
                }
            };
            for item in items {
                let item = match item {
                    Ok(item) => item,
                    Err(e) => {
                        self.block(dir.key.clone(), &dir.abs, &e.to_string());
                        break;
                    }
                };
                let abs = item.path();
                let Some(file_name) = item.file_name().to_str().map(str::to_owned) else {
                    self.warn(ScanWarning::InvalidName(abs));
                    continue;
                };
                if validate_name(&file_name).is_err() {
                    self.warn(ScanWarning::InvalidName(abs));
                    continue;
                }
                if self.req.skip_dirs.iter().any(|d| abs.starts_with(d)) {
                    continue;
                }
                let key = join_str(&dir.key, &file_name);
                let file_type = match item.file_type() {
                    Ok(t) => t,
                    Err(e) => {
                        self.block(key, &abs, &e.to_string());
                        continue;
                    }
                };
                if file_type.is_symlink() {
                    self.warn(ScanWarning::Symlink(abs));
                    continue;
                }
                let is_dir = file_type.is_dir();
                let rel_in_source = dir.rel_in_source.join(&file_name);
                if self.is_excluded(&rel_in_source, &abs, is_dir, &dir.ignores) {
                    self.result.excluded.insert(key);
                    continue;
                }
                let meta = match item.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        self.block(key, &abs, &e.to_string());
                        continue;
                    }
                };
                if is_dir {
                    let Ok(ignores) = self.load_syncignore(&abs, &key, dir.ignores.clone()) else {
                        continue;
                    };
                    self.result.entries.insert(key.clone(), folder_entry(&meta));
                    stack.push(Dir {
                        abs,
                        key,
                        rel_in_source,
                        ignores,
                    });
                } else if file_type.is_file() {
                    self.queue_file(key, &abs, &meta);
                } else {
                    self.warn(ScanWarning::Special(abs));
                }
            }
        }
    }

    /// Filtros da tarefa primeiro: um `.syncignore` não consegue reincluir arquivos
    /// sensíveis (RN-22). Depois os `.syncignore`, o mais fundo vence (RN-23).
    fn is_excluded(&self, rel: &Path, abs: &Path, is_dir: bool, ignores: &[Rc<Gitignore>]) -> bool {
        if self.req.filter.is_excluded(rel, is_dir) {
            return true;
        }
        for gi in ignores.iter().rev() {
            match gi.matched(abs, is_dir) {
                Match::Ignore(_) => return true,
                Match::Whitelist(_) => return false,
                Match::None => {}
            }
        }
        false
    }

    /// Acrescenta o `.syncignore` da pasta, se houver. Arquivo inválido bloqueia a
    /// pasta: sem saber o que excluir, é mais seguro não decidir nada (RN-27).
    fn load_syncignore(
        &mut self,
        dir: &Path,
        key: &str,
        mut ignores: Vec<Rc<Gitignore>>,
    ) -> Result<Vec<Rc<Gitignore>>, ()> {
        let file = dir.join(IGNORE_FILE_NAME);
        if !file.is_file() {
            return Ok(ignores);
        }
        let mut builder = GitignoreBuilder::new(dir);
        let built = match builder.add(&file) {
            None => builder.build().map_err(|e| e.to_string()),
            Some(e) => Err(e.to_string()),
        };
        match built {
            Ok(gi) => {
                ignores.push(Rc::new(gi));
                Ok(ignores)
            }
            Err(e) => {
                self.block(key.to_string(), &file, &e);
                Err(())
            }
        }
    }

    /// Registra o arquivo; reaproveita o hash do índice se tamanho e mtime não mudaram (RN-26).
    fn queue_file(&mut self, key: String, abs: &Path, meta: &Metadata) {
        let size = meta.len();
        let mtime_ns = mtime_ns(meta);
        let cached = self.index.get(&key).and_then(|i| {
            (i.kind == EntryKind::File && i.local_size == size && i.local_mtime_ns == mtime_ns)
                .then(|| i.local_hash.clone())
                .flatten()
        });
        if cached.is_none() {
            self.pending.push((key.clone(), abs.to_path_buf()));
        }
        self.result.entries.insert(
            key,
            LocalEntry {
                kind: EntryKind::File,
                size,
                mtime_ns,
                hash: cached,
            },
        );
    }

    /// RN-26: hash em paralelo, só do que mudou.
    fn hash_pending(&mut self) {
        let pending = std::mem::take(&mut self.pending);
        self.result.hashed_files = pending.len();
        let kind = self.req.hash_kind;
        let hashed: Vec<_> = pending
            .into_par_iter()
            .map(|(key, abs)| {
                let outcome = hash_file(&abs, kind);
                (key, abs, outcome)
            })
            .collect();
        for (key, abs, outcome) in hashed {
            match outcome {
                Ok(h) => {
                    if let Some(entry) = self.result.entries.get_mut(&key) {
                        entry.hash = Some(h);
                    }
                }
                Err(e) => {
                    self.result.entries.remove(&key);
                    self.block(key, &abs, &e.to_string());
                }
            }
        }
    }

    /// RN-27
    fn block(&mut self, key: String, path: &Path, error: &str) {
        self.result.blocked.insert(key);
        self.warn(ScanWarning::Unreadable {
            path: path.to_path_buf(),
            error: error.to_string(),
        });
    }

    fn warn(&mut self, warning: ScanWarning) {
        self.result.warnings.push(warning);
    }
}

fn folder_entry(meta: &Metadata) -> LocalEntry {
    LocalEntry {
        kind: EntryKind::Folder,
        size: 0,
        mtime_ns: mtime_ns(meta),
        hash: None,
    }
}

pub fn mtime_ns(meta: &Metadata) -> i64 {
    match meta.modified().map(|t| t.duration_since(UNIX_EPOCH)) {
        Ok(Ok(d)) => i64::try_from(d.as_nanos()).unwrap_or(i64::MAX),
        Ok(Err(before_epoch)) => {
            -i64::try_from(before_epoch.duration().as_nanos()).unwrap_or(i64::MAX)
        }
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn request(sources: Vec<PathBuf>, exclude: &[&str], skip: Vec<PathBuf>) -> ScanRequest {
        let exclude: Vec<String> = exclude.iter().map(|s| s.to_string()).collect();
        ScanRequest {
            sources,
            filter: Filter::new(&exclude, false).unwrap(),
            skip_dirs: skip,
            hash_kind: HashKind::Md5,
        }
    }

    fn keys(result: &ScanResult) -> Vec<&str> {
        result.entries.keys().map(String::as_str).collect()
    }

    #[test]
    fn aplica_filtros_links_e_pastas_do_app() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join("a.txt"), "a");
        write(&src.join("sub/b.txt"), "b");
        write(&src.join(".DS_Store"), "x");
        write(&src.join("node_modules/x.js"), "x");
        write(&src.join(".env"), "SECRET=1");
        write(&src.join("app-data/state.db"), "x");
        #[cfg(unix)]
        std::os::unix::fs::symlink(src.join("a.txt"), src.join("link.txt")).unwrap();

        let req = request(
            vec![src.clone()],
            &["node_modules/"],
            vec![src.join("app-data")],
        );
        let result = scan(&req, &BTreeMap::new());

        assert_eq!(
            keys(&result),
            ["Docs", "Docs/a.txt", "Docs/sub", "Docs/sub/b.txt"]
        );
        assert_eq!(
            result.entries["Docs/a.txt"].hash.as_deref(),
            Some("0cc175b9c0f1b6a831c399e269772661")
        );
        assert_eq!(result.sources[0].files, 2);
        assert!(result.blocked.is_empty());
        // RN-28: exclusões ficam registradas (pastas do app não contam como exclusão).
        let excluded: Vec<_> = result.excluded.iter().map(String::as_str).collect();
        assert_eq!(
            excluded,
            ["Docs/.DS_Store", "Docs/.env", "Docs/node_modules"]
        );
        #[cfg(unix)]
        assert!(
            result
                .warnings
                .iter()
                .any(|w| matches!(w, ScanWarning::Symlink(_)))
        );
    }

    #[test]
    fn rn23_syncignore_aninhado_e_negacao() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join(".syncignore"), "*.log\n");
        write(&src.join("a.log"), "x");
        write(&src.join("sub/.syncignore"), "!manter.log\n");
        write(&src.join("sub/manter.log"), "x");
        write(&src.join("sub/outro.log"), "x");

        let result = scan(&request(vec![src], &[], vec![]), &BTreeMap::new());
        assert!(result.entries.contains_key("Docs/sub/manter.log"));
        assert!(!result.entries.contains_key("Docs/sub/outro.log"));
        assert!(result.excluded.contains("Docs/a.log"));
    }

    #[test]
    fn rn22_syncignore_nao_reinclui_sensiveis() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join(".syncignore"), "!.env\n");
        write(&src.join(".env"), "SECRET=1");
        let result = scan(&request(vec![src], &[], vec![]), &BTreeMap::new());
        assert!(!result.entries.contains_key("Docs/.env"));
    }

    #[test]
    fn rn22_fonte_sensivel_escolhida_diretamente_e_ignorada() {
        let tmp = tempfile::tempdir().unwrap();
        write(&tmp.path().join(".ssh/config"), "Host x");
        write(&tmp.path().join(".env"), "SECRET=1");
        let req = request(
            vec![tmp.path().join(".ssh"), tmp.path().join(".env")],
            &[],
            vec![],
        );
        let result = scan(&req, &BTreeMap::new());
        assert!(result.entries.is_empty());
        assert_eq!(result.excluded.len(), 2);
    }

    #[test]
    fn rn27_syncignore_invalido_bloqueia_a_pasta() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join("ok.txt"), "x");
        write(&src.join("sub/.syncignore"), "[z-a]\n");
        write(&src.join("sub/x.txt"), "x");
        let result = scan(&request(vec![src], &[], vec![]), &BTreeMap::new());
        assert!(result.blocked.contains("Docs/sub"));
        assert!(result.entries.contains_key("Docs/ok.txt"));
        assert!(!result.entries.contains_key("Docs/sub/x.txt"));
    }

    #[test]
    fn rn26_reaproveita_hash_do_indice() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join("a.txt"), "a");
        let meta = fs::metadata(src.join("a.txt")).unwrap();

        let mut index = BTreeMap::new();
        index.insert(
            "Docs/a.txt".to_string(),
            IndexEntry {
                path: "Docs/a.txt".into(),
                kind: EntryKind::File,
                local_size: meta.len(),
                local_mtime_ns: mtime_ns(&meta),
                local_hash: Some("do-indice".into()),
                remote_id: "r".into(),
                remote_hash: Some("do-indice".into()),
                synced_at: Utc::now(),
            },
        );
        let result = scan(&request(vec![src], &[], vec![]), &index);
        assert_eq!(result.hashed_files, 0);
        assert_eq!(
            result.entries["Docs/a.txt"].hash.as_deref(),
            Some("do-indice")
        );
    }

    #[test]
    fn fonte_arquivo_e_fonte_ausente() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("notas.md");
        write(&file, "n");
        let missing = tmp.path().join("NaoExiste");
        let result = scan(&request(vec![file, missing], &[], vec![]), &BTreeMap::new());
        assert!(result.entries.contains_key("notas.md"));
        assert!(result.sources[0].exists && result.sources[0].files == 1);
        assert!(!result.sources[1].exists);
    }

    #[cfg(unix)]
    #[test]
    fn rn27_pasta_ilegivel_fica_bloqueada() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        write(&src.join("fechada/x.txt"), "x");
        let closed = src.join("fechada");
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o000)).unwrap();
        // Root ignora permissões; nesse caso o teste não se aplica.
        let readable = fs::read_dir(&closed).is_ok();

        let result = scan(&request(vec![src], &[], vec![]), &BTreeMap::new());
        fs::set_permissions(&closed, fs::Permissions::from_mode(0o755)).unwrap();
        if !readable {
            assert!(result.blocked.contains("Docs/fechada"));
        }
    }
}
