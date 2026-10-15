//! Executor: aplica um [`Plan`]. Não decide nada (RN-30) — só executa, na ordem da RN-39:
//! pastas → transferências (em paralelo) → lixeira de arquivos → lixeira de pastas.
//!
//! - Cada item confirmado vai para o índice na hora (RN-38): uma execução interrompida
//!   continua de onde parou na próxima.
//! - Erro de um item não para os outros (RN-97); erro da conta para tudo (RN-98, RN-99).
//! - Arquivo que muda durante o envio é reenviado na próxima execução (RN-25).

use std::collections::{BTreeMap, HashMap};
use std::fs::Metadata;
use std::future::Future;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use chrono::Utc;
use futures_util::stream::{self, StreamExt};
use serde::Serialize;
use uuid::Uuid;

use crate::domain::rel_path::parent_str;
use crate::domain::{Action, EntryKind, IndexEntry, Plan, RelPath};
use crate::error::{CoreError, CoreResult};
use crate::provider::{ErrorScope, ProviderError, ProviderResult, RemoteEntry, StorageProvider};
use crate::state::StateDb;
use crate::sync::remote_tree::resolve_path;
use crate::sync::retry::RetryPolicy;
use crate::sync::scanner::{ScanResult, mtime_ns};

/// Marca no índice um arquivo que mudou durante o envio: força novo hash e reenvio (RN-25).
pub const UNSTABLE_MTIME: i64 = -1;
/// RN-99: itens seguidos esgotando tentativas por limite de taxa antes de parar.
const RATE_LIMIT_STREAK: u32 = 3;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    pub total: usize,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub path: String,
    pub error: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionReport {
    pub folders_created: u32,
    pub uploaded: u32,
    pub updated: u32,
    pub conflicts: u32,
    pub adopted: u32,
    pub trashed: u32,
    pub forgotten: u32,
    pub bytes_sent: u64,
    /// RN-25: mudaram durante o envio; vão de novo na próxima execução.
    pub unstable: u32,
    pub failures: Vec<Failure>,
    pub cancelled: bool,
    /// Erro que afetou a conta inteira (RN-98, RN-99): a execução parou aqui.
    pub aborted: Option<String>,
}

impl ExecutionReport {
    pub fn succeeded_items(&self) -> u32 {
        self.folders_created
            + self.uploaded
            + self.updated
            + self.conflicts
            + self.adopted
            + self.trashed
    }
}

pub struct Execution<'a> {
    pub job_id: Uuid,
    /// Pasta de destino, relativa à raiz do app (criada se faltar).
    pub dest: &'a RelPath,
    pub plan: &'a Plan,
    pub scan: &'a ScanResult,
    pub remote: &'a BTreeMap<String, RemoteEntry>,
    pub parallelism: usize,
    /// Tentativas do executor; o provedor já faz as próprias (ex.: backoff HTTP).
    pub retry: RetryPolicy,
    pub cancel: &'a AtomicBool,
    pub progress: &'a (dyn Fn(&Progress) + Send + Sync),
}

#[derive(Default)]
struct Shared {
    report: ExecutionReport,
    /// Caminho relativo → ID remoto das pastas conhecidas.
    folders: HashMap<String, String>,
    progress: Progress,
    rate_limit_streak: u32,
}

struct Ctx<'a> {
    provider: &'a dyn StorageProvider,
    state: &'a Mutex<StateDb>,
    run: &'a Execution<'a>,
    shared: Mutex<Shared>,
}

pub async fn execute(
    provider: &dyn StorageProvider,
    state: &Mutex<StateDb>,
    run: &Execution<'_>,
) -> CoreResult<ExecutionReport> {
    let dest = resolve_path(provider, run.dest, true)
        .await?
        .ok_or_else(|| CoreError::Destination(format!("não foi possível criar '{}'", run.dest)))?;

    let mut shared = Shared::default();
    shared.folders.insert(String::new(), dest.id);
    for (path, entry) in run.remote {
        if entry.kind == EntryKind::Folder {
            shared.folders.insert(path.clone(), entry.id.clone());
        }
    }
    let actions: Vec<&Action> = run
        .plan
        .actions
        .iter()
        .filter(|a| !matches!(a, Action::Skip { .. }))
        .collect();
    shared.progress.total = actions.len();
    shared.progress.bytes_total = run.plan.stats.bytes_to_send;

    let ctx = Ctx {
        provider,
        state,
        run,
        shared: Mutex::new(shared),
    };
    ctx.emit(None);

    // RN-39: 1) pastas, em ordem (pai antes do filho).
    for action in actions
        .iter()
        .filter(|a| matches!(a, Action::CreateFolder { .. }))
    {
        if ctx.should_stop() {
            break;
        }
        ctx.create_folder(action).await;
    }
    // 2) transferências e adoções, em paralelo.
    let transfers = actions.iter().filter(|a| {
        matches!(
            a,
            Action::Upload { .. }
                | Action::Update { .. }
                | Action::ResolveConflict { .. }
                | Action::Adopt { .. }
        )
    });
    stream::iter(transfers)
        .for_each_concurrent(run.parallelism.max(1), |a| ctx.transfer(a))
        .await;
    // 3) lixeira de arquivos; 4) de pastas, mais fundas primeiro (o plano já vem ordenado).
    let trash_files = actions
        .iter()
        .filter(|a| matches!(a, Action::Trash { kind, .. } if *kind == EntryKind::File));
    stream::iter(trash_files)
        .for_each_concurrent(run.parallelism.max(1), |a| ctx.trash(a))
        .await;
    for action in actions
        .iter()
        .filter(|a| matches!(a, Action::Trash { kind, .. } if *kind == EntryKind::Folder))
    {
        ctx.trash(action).await;
    }
    // 5) esquecer do índice.
    let forget: Vec<String> = actions
        .iter()
        .filter_map(|a| match a {
            Action::ForgetIndex { path } => Some(path.clone()),
            _ => None,
        })
        .collect();
    if !forget.is_empty() && !ctx.should_stop() {
        let result = ctx.lock_state().remove_index(run.job_id, &forget);
        let mut shared = ctx.lock();
        match result {
            Ok(()) => {
                shared.report.forgotten += forget.len() as u32;
                shared.progress.done += forget.len();
            }
            Err(e) => shared.report.aborted = Some(format!("falha ao gravar o índice: {e}")),
        }
    }

    ctx.emit(None);
    let mut report = ctx
        .shared
        .into_inner()
        .unwrap_or_else(PoisonError::into_inner)
        .report;
    report.cancelled = run.cancel.load(Ordering::Relaxed);
    Ok(report)
}

impl Ctx<'_> {
    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, StateDb> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn should_stop(&self) -> bool {
        self.run.cancel.load(Ordering::Relaxed) || self.lock().report.aborted.is_some()
    }

    fn emit(&self, current: Option<&str>) {
        let snapshot = {
            let mut shared = self.lock();
            shared.progress.current = current.map(str::to_owned);
            shared.progress.clone()
        };
        (self.run.progress)(&snapshot);
    }

    fn finish_item(&self, bytes: u64) {
        {
            let mut shared = self.lock();
            shared.progress.done += 1;
            shared.progress.bytes_done += bytes;
            shared.rate_limit_streak = 0;
        }
        self.emit(None);
    }

    /// RN-97/98/99: decide se o erro fica no item ou para a execução inteira.
    fn fail(&self, path: &str, err: &ProviderError) {
        {
            let mut shared = self.lock();
            shared.progress.done += 1;
            match err.scope() {
                ErrorScope::Account => shared.report.aborted = Some(err.to_string()),
                _ => {
                    if matches!(err, ProviderError::RateLimited { .. }) {
                        shared.rate_limit_streak += 1;
                        if shared.rate_limit_streak >= RATE_LIMIT_STREAK {
                            shared.report.aborted = Some(
                                "limite de requisições do provedor persistente — tente mais tarde"
                                    .into(),
                            );
                        }
                    }
                    shared.report.failures.push(Failure {
                        path: path.to_string(),
                        error: err.to_string(),
                    });
                }
            }
        }
        tracing::warn!(path, "item falhou: {err}");
        self.emit(None);
    }

    fn record(&self, entry: IndexEntry) -> bool {
        let result = self.lock_state().upsert_index(self.run.job_id, &[entry]);
        if let Err(e) = result {
            self.lock().report.aborted = Some(format!("falha ao gravar o índice: {e}"));
            return false;
        }
        true
    }

    async fn attempt<T, F, Fut>(&self, mut op: F) -> ProviderResult<T>
    where
        F: FnMut() -> Fut,
        Fut: Future<Output = ProviderResult<T>>,
    {
        let mut attempt = 0;
        loop {
            attempt += 1;
            match op().await {
                Ok(value) => return Ok(value),
                Err(err) => match self.run.retry.next_delay(attempt, &err, jitter()) {
                    Some(delay) => tokio::time::sleep(delay).await,
                    None => return Err(err),
                },
            }
        }
    }

    fn folder_id(&self, parent: &str) -> Option<String> {
        self.lock().folders.get(parent).cloned()
    }

    async fn create_folder(&self, action: &Action) {
        let Action::CreateFolder { path } = action else {
            return;
        };
        let name = file_name(path);
        let Some(parent_id) = self.folder_id(parent_str(path)) else {
            return self.fail(path, &missing_parent());
        };
        self.emit(Some(path));
        match self
            .attempt(|| self.provider.create_folder(&parent_id, name))
            .await
        {
            Ok(entry) => {
                let local = self.run.scan.entries.get(path);
                let indexed = self.record(IndexEntry {
                    path: path.clone(),
                    kind: EntryKind::Folder,
                    local_size: 0,
                    local_mtime_ns: local.map_or(0, |l| l.mtime_ns),
                    local_hash: None,
                    remote_id: entry.id.clone(),
                    remote_hash: None,
                    synced_at: Utc::now(),
                    remote_modified_at: entry.modified_at,
                    remote_modified_by: entry.modified_by.clone(),
                });
                if indexed {
                    let mut shared = self.lock();
                    shared.folders.insert(path.clone(), entry.id);
                    shared.report.folders_created += 1;
                }
                self.finish_item(0);
            }
            Err(e) => self.fail(path, &e),
        }
    }

    async fn transfer(&self, action: &Action) {
        if self.should_stop() {
            return;
        }
        let path = action.path();
        if let Action::Adopt { remote_id, .. } = action {
            return self.adopt(path, remote_id);
        }
        let Some(abs) = self.run.scan.local_path(path) else {
            return self.fail(
                path,
                &ProviderError::Other("caminho local desconhecido".into()),
            );
        };
        let Some(parent_id) = self.folder_id(parent_str(path)) else {
            return self.fail(path, &missing_parent());
        };
        let name = file_name(path);
        let before = match std::fs::metadata(&abs) {
            Ok(m) => m,
            Err(e) => return self.fail(path, &ProviderError::Io(e)),
        };
        self.emit(Some(path));

        let result = match action {
            Action::Upload { .. } => self.upload_new(&parent_id, name, &abs).await,
            Action::Update { remote_id, .. } => {
                self.attempt(|| self.provider.upload_replace(remote_id, &abs))
                    .await
            }
            Action::ResolveConflict {
                remote_id,
                conflict_name,
                ..
            } => {
                // RN-35: preserva a versão remota com outro nome antes de enviar a local.
                match self
                    .attempt(|| self.provider.rename(remote_id, conflict_name))
                    .await
                {
                    Ok(_) => self.upload_new(&parent_id, name, &abs).await,
                    Err(e) => Err(e),
                }
            }
            _ => return,
        };
        let entry = match result {
            Ok(entry) => entry,
            Err(e) => return self.fail(path, &e),
        };

        // RN-25: se o arquivo mudou durante o envio, o índice não o marca como sincronizado.
        let after = std::fs::metadata(&abs).ok();
        let stable = after.as_ref().is_some_and(|a| same_version(&before, a));
        let size = entry.size.unwrap_or(before.len());
        let indexed = self.record(IndexEntry {
            path: path.to_string(),
            kind: EntryKind::File,
            local_size: after.as_ref().map_or(before.len(), Metadata::len),
            local_mtime_ns: if stable {
                mtime_ns(&before)
            } else {
                UNSTABLE_MTIME
            },
            local_hash: entry.hash.clone(),
            remote_id: entry.id.clone(),
            remote_hash: entry.hash.clone(),
            synced_at: Utc::now(),
            remote_modified_at: entry.modified_at,
            remote_modified_by: entry.modified_by.clone(),
        });
        if indexed {
            let mut shared = self.lock();
            let report = &mut shared.report;
            match action {
                Action::Upload { .. } => report.uploaded += 1,
                Action::Update { .. } => report.updated += 1,
                _ => report.conflicts += 1,
            }
            report.bytes_sent += size;
            if !stable {
                report.unstable += 1;
            }
        }
        self.finish_item(size);
    }

    /// Envio novo. Se uma tentativa falhar sem resposta, o arquivo pode ter chegado mesmo
    /// assim: antes de tentar de novo, procura por ele para não criar duplicata (RN-34).
    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        abs: &Path,
    ) -> ProviderResult<RemoteEntry> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let err = match self.provider.upload_new(parent_id, name, abs).await {
                Ok(entry) => return Ok(entry),
                Err(e) => e,
            };
            let Some(delay) = self.run.retry.next_delay(attempt, &err, jitter()) else {
                return Err(err);
            };
            tokio::time::sleep(delay).await;
            if !matches!(err, ProviderError::ChecksumMismatch) {
                let siblings = self.provider.list_children(parent_id).await?;
                if let Some(existing) = siblings.into_iter().find(|s| s.name == name) {
                    return Ok(existing);
                }
            }
        }
    }

    fn adopt(&self, path: &str, remote_id: &str) {
        let (Some(local), Some(remote)) =
            (self.run.scan.entries.get(path), self.run.remote.get(path))
        else {
            return self.fail(
                path,
                &ProviderError::Other("item sumiu antes da adoção".into()),
            );
        };
        let indexed = self.record(IndexEntry {
            path: path.to_string(),
            kind: local.kind,
            local_size: local.size,
            local_mtime_ns: local.mtime_ns,
            local_hash: local.hash.clone(),
            remote_id: remote_id.to_string(),
            remote_hash: remote.hash.clone(),
            synced_at: Utc::now(),
            remote_modified_at: remote.modified_at,
            remote_modified_by: remote.modified_by.clone(),
        });
        if indexed {
            self.lock().report.adopted += 1;
        }
        self.finish_item(0);
    }

    async fn trash(&self, action: &Action) {
        let Action::Trash {
            path, remote_id, ..
        } = action
        else {
            return;
        };
        if self.should_stop() {
            return;
        }
        self.emit(Some(path));
        match self.attempt(|| self.provider.trash(remote_id)).await {
            Ok(()) => {
                let result = self
                    .lock_state()
                    .remove_index(self.run.job_id, std::slice::from_ref(path));
                match result {
                    Ok(()) => self.lock().report.trashed += 1,
                    Err(e) => {
                        self.lock().report.aborted = Some(format!("falha ao gravar o índice: {e}"));
                    }
                }
                self.finish_item(0);
            }
            Err(e) => self.fail(path, &e),
        }
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn missing_parent() -> ProviderError {
    ProviderError::Other("a pasta de destino deste item não foi criada".into())
}

fn same_version(a: &Metadata, b: &Metadata) -> bool {
    a.len() == b.len() && mtime_ns(a) == mtime_ns(b)
}

/// Jitter para o backoff, a partir do gerador do `uuid` (aleatoriedade do SO).
fn jitter() -> f64 {
    (Uuid::new_v4().as_u128() >> 75) as f64 / (1u64 << 53) as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{SkipReason, SyncMode};
    use crate::filters::Filter;
    use crate::hash::HashKind;
    use crate::provider::StorageQuota;
    use crate::providers::LocalProvider;
    use crate::sync::planner::{self, PlanInput};
    use crate::sync::remote_tree::load_remote_tree;
    use crate::sync::scanner::{self, ScanRequest};
    use async_trait::async_trait;
    use std::path::PathBuf;
    use std::time::Duration;

    struct Env {
        _tmp: tempfile::TempDir,
        src: PathBuf,
        dest_root: PathBuf,
        state: Mutex<StateDb>,
        job: Uuid,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("Docs");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), "a").unwrap();
        std::fs::write(src.join("sub/b.txt"), "b").unwrap();
        let dest_root = tmp.path().join("hd");
        std::fs::create_dir_all(&dest_root).unwrap();
        LocalProvider::initialize(&dest_root).unwrap();
        Env {
            src,
            dest_root,
            state: Mutex::new(StateDb::open_in_memory().unwrap()),
            job: Uuid::new_v4(),
            _tmp: tmp,
        }
    }

    /// Escaneia, planeja e executa uma rodada completa.
    async fn round(
        env: &Env,
        provider: &dyn StorageProvider,
        mode: SyncMode,
        cancel: bool,
    ) -> (Plan, ExecutionReport) {
        let dest = RelPath::parse("Backup").unwrap();
        let index = env.state.lock().unwrap().load_index(env.job).unwrap();
        let scan = scanner::scan(
            &ScanRequest {
                sources: vec![env.src.clone()],
                filter: Filter::new(&[], false).unwrap(),
                skip_dirs: vec![],
                hash_kind: Some(HashKind::Md5),
            },
            &index,
        );
        let remote = load_remote_tree(provider, &dest).await.unwrap();
        let blocked: BTreeMap<String, SkipReason> = BTreeMap::new();
        let plan = planner::plan(&PlanInput {
            mode,
            local: &scan.entries,
            index: &index,
            remote: &remote.entries,
            blocked: &blocked,
            excluded: &scan.excluded,
            conflict_stamp: "2026-10-10 12-00",
        });
        let cancel = AtomicBool::new(cancel);
        let report = execute(
            provider,
            &env.state,
            &Execution {
                job_id: env.job,
                dest: &dest,
                plan: &plan,
                scan: &scan,
                remote: &remote.entries,
                parallelism: 4,
                retry: RetryPolicy {
                    max_attempts: 2,
                    base_delay: Duration::from_millis(1),
                    max_delay: Duration::from_millis(2),
                },
                cancel: &cancel,
                progress: &|_| {},
            },
        )
        .await
        .unwrap();
        (plan, report)
    }

    #[tokio::test]
    async fn primeira_execucao_envia_tudo_e_a_segunda_nao_faz_nada() {
        let env = env();
        let p = LocalProvider::new(&env.dest_root);
        let (_, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(report.uploaded, 2);
        assert_eq!(report.folders_created, 2);
        assert!(report.failures.is_empty() && report.aborted.is_none());
        assert_eq!(
            std::fs::read_to_string(env.dest_root.join("Backup/Docs/sub/b.txt")).unwrap(),
            "b"
        );
        // RN-38: tudo indexado — a segunda rodada não tem nada a fazer.
        let (plan, _) = round(&env, &p, SyncMode::Upload, false).await;
        assert!(plan.actions.is_empty(), "{:?}", plan.actions);
    }

    #[tokio::test]
    async fn alteracao_local_vira_atualizacao() {
        let env = env();
        let p = LocalProvider::new(&env.dest_root);
        round(&env, &p, SyncMode::Upload, false).await;
        std::fs::write(env.src.join("a.txt"), "novo conteúdo").unwrap();
        let (_, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(report.updated, 1);
        assert_eq!(
            std::fs::read_to_string(env.dest_root.join("Backup/Docs/a.txt")).unwrap(),
            "novo conteúdo"
        );
    }

    #[tokio::test]
    async fn espelho_manda_exclusoes_para_a_lixeira() {
        let env = env();
        let p = LocalProvider::new(&env.dest_root);
        round(&env, &p, SyncMode::Mirror, false).await;
        std::fs::remove_file(env.src.join("sub/b.txt")).unwrap();
        let (_, report) = round(&env, &p, SyncMode::Mirror, false).await;
        assert_eq!(report.trashed, 1);
        assert!(!env.dest_root.join("Backup/Docs/sub/b.txt").exists());
        // RN-44: foi para a lixeira, não apagado.
        assert!(env.dest_root.join(".syncly-trash").exists());
    }

    #[tokio::test]
    async fn rn95_cancelado_antes_de_comecar_nao_envia_nada() {
        let env = env();
        let p = LocalProvider::new(&env.dest_root);
        let (_, report) = round(&env, &p, SyncMode::Upload, true).await;
        assert!(report.cancelled);
        assert_eq!(report.uploaded, 0);
    }

    /// Provedor local com falhas programadas.
    struct Faulty {
        inner: LocalProvider,
        fail_name: Option<&'static str>,
        error: fn() -> ProviderError,
        /// Simula o arquivo (nome, caminho local) mudando no meio do próprio envio.
        touch_during_upload: Option<(&'static str, PathBuf)>,
    }

    #[async_trait]
    impl StorageProvider for Faulty {
        fn id(&self) -> &'static str {
            "faulty"
        }
        fn hash_kind(&self) -> HashKind {
            HashKind::Md5
        }
        async fn app_root(&self) -> ProviderResult<RemoteEntry> {
            self.inner.app_root().await
        }
        async fn list_children(&self, f: &str) -> ProviderResult<Vec<RemoteEntry>> {
            self.inner.list_children(f).await
        }
        async fn get(&self, id: &str) -> ProviderResult<Option<RemoteEntry>> {
            self.inner.get(id).await
        }
        async fn create_folder(&self, p: &str, n: &str) -> ProviderResult<RemoteEntry> {
            self.inner.create_folder(p, n).await
        }
        async fn upload_new(&self, p: &str, n: &str, l: &Path) -> ProviderResult<RemoteEntry> {
            if self.fail_name == Some(n) {
                return Err((self.error)());
            }
            let entry = self.inner.upload_new(p, n, l).await;
            if let Some((name, path)) = &self.touch_during_upload
                && *name == n
            {
                std::fs::write(path, "mudou durante o envio").unwrap();
            }
            entry
        }
        async fn upload_replace(&self, id: &str, l: &Path) -> ProviderResult<RemoteEntry> {
            self.inner.upload_replace(id, l).await
        }
        async fn download(&self, id: &str, d: &Path) -> ProviderResult<()> {
            self.inner.download(id, d).await
        }
        async fn rename(&self, id: &str, n: &str) -> ProviderResult<RemoteEntry> {
            self.inner.rename(id, n).await
        }
        async fn trash(&self, id: &str) -> ProviderResult<()> {
            self.inner.trash(id).await
        }
        async fn quota(&self) -> ProviderResult<Option<StorageQuota>> {
            Ok(None)
        }
    }

    fn faulty(env: &Env, fail_name: Option<&'static str>, error: fn() -> ProviderError) -> Faulty {
        Faulty {
            inner: LocalProvider::new(&env.dest_root),
            fail_name,
            error,
            touch_during_upload: None,
        }
    }

    #[tokio::test]
    async fn rn97_falha_de_um_item_nao_para_os_outros() {
        let env = env();
        let p = faulty(&env, Some("a.txt"), || {
            ProviderError::PermissionDenied("x".into())
        });
        let (_, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(report.uploaded, 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].path, "Docs/a.txt");
        assert!(report.aborted.is_none());
        // A próxima rodada tenta só o que falhou.
        let p = LocalProvider::new(&env.dest_root);
        let (plan, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(plan.stats.uploads, 1);
        assert_eq!(report.uploaded, 1);
    }

    #[tokio::test]
    async fn rn98_erro_da_conta_para_a_execucao() {
        let env = env();
        let p = faulty(&env, Some("a.txt"), || ProviderError::AuthExpired);
        let (_, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert!(report.aborted.is_some());
        assert!(report.failures.is_empty());
    }

    #[tokio::test]
    async fn rn25_arquivo_que_muda_durante_o_envio_e_reenviado() {
        let env = env();
        let mut p = faulty(&env, None, || ProviderError::Other("x".into()));
        p.touch_during_upload = Some(("a.txt", env.src.join("a.txt")));
        let (_, report) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(report.unstable, 1);
        // Na próxima rodada, o conteúdo novo é enviado.
        let p = LocalProvider::new(&env.dest_root);
        let (plan, _) = round(&env, &p, SyncMode::Upload, false).await;
        assert_eq!(plan.stats.updates, 1, "{:?}", plan.actions);
        assert_eq!(
            std::fs::read_to_string(env.dest_root.join("Backup/Docs/a.txt")).unwrap(),
            "mudou durante o envio"
        );
    }
}
