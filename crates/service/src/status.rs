//! Status e última modificação (RN-100 a RN-103). Sem rede: computador (varredura sem
//! hash) + índice. "Atualizar do destino" lista o destino e guarda uma foto, usada até a
//! próxima execução da tarefa.

use std::collections::BTreeMap;
use std::time::Instant;

use chrono::Utc;
use syncly_core::domain::{Delivery, EntryKind, RelPath};
use syncly_core::filters::Filter;
use syncly_core::sync::remote_tree::load_remote_tree;
use syncly_core::sync::scanner::{self, ScanRequest};
use syncly_core::sync::status::{self, RemoteInfo, RemoteSnapshot, StatusInput};
use uuid::Uuid;

use crate::dto::{JobStatus, MAX_STATUS_ITEMS};
use crate::error::{ServiceError, ServiceResult};
use crate::service::{Service, elapsed_ms};

impl Service {
    /// Situação dos itens de uma pasta da tarefa (`""` = raiz) e os totais dela.
    pub async fn job_status(&self, id: Uuid, folder: &str) -> ServiceResult<JobStatus> {
        let started = Instant::now();
        let folder = RelPath::parse(folder).map_err(ServiceError::Invalid)?;
        let loaded = self.load_job(id)?;
        let state = self.state()?;
        let index = state.load_index(id)?;
        let snapshot = state.load_snapshot(id)?;
        drop(state);

        let synced: Vec<u64> = index
            .values()
            .filter(|e| e.kind == EntryKind::File)
            .map(|e| e.local_size)
            .collect();
        let mut out = JobStatus {
            job_id: id,
            folder: folder.to_string(),
            per_file: loaded.sync.delivery == Delivery::Files,
            items: Vec::new(),
            total_items: 0,
            counts: Default::default(),
            local_files: 0,
            local_bytes: 0,
            synced_files: synced.len() as u64,
            synced_bytes: synced.iter().sum(),
            remote_checked_at: snapshot.as_ref().map(|s| s.fetched_at),
            duration_ms: 0,
        };
        // Compactadas e as que recebem: o status delas são as versões e o histórico.
        if !out.per_file {
            out.duration_ms = elapsed_ms(started);
            return Ok(out);
        }

        // Mesmos filtros da execução; sem hash — tamanho e data bastam (RN-103).
        let request = ScanRequest {
            sources: loaded.job.sources.clone(),
            filter: Filter::new(&loaded.job.exclude, loaded.job.include_sensitive)
                .map_err(ServiceError::Invalid)?,
            skip_dirs: self.paths.app_dirs(),
            hash_kind: None,
        };
        let scan =
            tokio::task::spawn_blocking(move || scanner::scan(&request, &Default::default()))
                .await
                .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let kind_of = |key: &str| {
            if scan.local_path(key).is_some_and(|p| p.is_dir()) {
                EntryKind::Folder
            } else {
                EntryKind::File
            }
        };
        let mut ignored = BTreeMap::new();
        for key in &scan.excluded {
            ignored.insert(key.clone(), (kind_of(key), "excluído pelos filtros"));
        }
        // RN-27
        for key in &scan.blocked {
            ignored.insert(key.clone(), (kind_of(key), "não pôde ser lido"));
        }

        let all = status::compute(&StatusInput {
            local: &scan.entries,
            index: &index,
            snapshot: snapshot.as_ref(),
            ignored: &ignored,
        });
        let children = status::children(&all, folder.as_str());
        let locals = scan.entries.values().filter(|e| e.kind == EntryKind::File);
        out.local_files = locals.clone().count() as u64;
        out.local_bytes = locals.map(|e| e.size).sum();
        out.counts = status::counts(&all);
        out.total_items = children.len();
        out.items = children
            .into_iter()
            .take(MAX_STATUS_ITEMS)
            .cloned()
            .collect();
        out.duration_ms = elapsed_ms(started);
        Ok(out)
    }

    /// RN-103: "Atualizar do destino" — lista o destino agora e guarda a foto. Mostra o que
    /// mudou lá (inclusive por outras pessoas) e os conflitos.
    pub async fn refresh_job_status(&self, id: Uuid, folder: &str) -> ServiceResult<JobStatus> {
        let loaded = self.load_job(id)?;
        if loaded.sync.delivery != Delivery::Files {
            return Err(ServiceError::Invalid(
                "só tarefas que enviam arquivo por arquivo têm status por arquivo".into(),
            ));
        }
        let provider = self.provider_for(&loaded.account).await?;
        let tree = load_remote_tree(provider.as_ref(), &loaded.sync.destination.path).await?;
        let snapshot = RemoteSnapshot {
            fetched_at: Utc::now(),
            entries: tree
                .entries
                .iter()
                .map(|(path, entry)| (path.clone(), RemoteInfo::from(entry)))
                .collect(),
        };
        self.state()?.save_snapshot(id, &snapshot)?;
        tracing::info!(job = %id, items = snapshot.entries.len(), "destino consultado");
        self.job_status(id, folder).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{JobDraft, SyncDraft};
    use crate::service::RunOptions;
    use crate::test_support::{FAKE_NEXT_RUN, FakeOs, service_with_os};
    use std::path::PathBuf;
    use std::sync::Arc;
    use syncly_core::domain::{Retention, RunTrigger, Schedule, SyncMode};
    use syncly_core::sync::status::Situation;

    struct Setup {
        _tmp: tempfile::TempDir,
        service: Service,
        source: PathBuf,
        dest: PathBuf,
        job: Uuid,
    }

    fn setup(delivery: Delivery) -> Setup {
        let tmp = tempfile::tempdir().unwrap();
        let service = service_with_os(
            &tmp.path().join("app"),
            "http://127.0.0.1:9",
            Arc::new(FakeOs::default()),
        );
        let source = tmp.path().join("Docs");
        std::fs::create_dir_all(source.join("sub")).unwrap();
        std::fs::write(source.join("a.txt"), "a").unwrap();
        std::fs::write(source.join("sub/b.txt"), "bb").unwrap();
        std::fs::write(source.join("debug.log"), "x").unwrap();
        let hd = tmp.path().join("hd");
        std::fs::create_dir_all(&hd).unwrap();
        let account = service.add_local_account("HD".into(), hd.clone()).unwrap();
        let job = service
            .save_job(JobDraft {
                id: None,
                name: "Docs".into(),
                enabled: true,
                sources: vec![source.clone()],
                exclude: vec!["*.log".into()],
                include_sensitive: false,
                sync: Some(SyncDraft {
                    account_id: account.id,
                    path: "Backup".into(),
                    delivery,
                    mode: SyncMode::Upload,
                    schedule: Some(Schedule::Daily {
                        at: "02:30".to_string().try_into().unwrap(),
                    }),
                }),
            })
            .unwrap()
            .id;
        Setup {
            _tmp: tmp,
            service,
            source,
            dest: hd.join("Backup"),
            job,
        }
    }

    async fn run(s: &Setup) {
        let options = RunOptions {
            confirm: true,
            trigger: RunTrigger::Manual,
        };
        s.service.run_job(s.job, options, |_| {}).await.unwrap();
    }

    fn situation(status: &JobStatus, name: &str) -> (Situation, Option<&'static str>) {
        let item = status.items.iter().find(|i| i.name == name).unwrap();
        (item.situation, item.detail)
    }

    #[tokio::test]
    async fn rn101_rn103_status_sem_rede_e_com_o_destino() {
        let s = setup(Delivery::Files);
        // Antes do primeiro sync: tudo pendente; o filtrado aparece como ignorado.
        let status = s.service.job_status(s.job, "Docs").await.unwrap();
        assert_eq!(
            situation(&status, "a.txt"),
            (Situation::Pending, Some("novo"))
        );
        assert_eq!(situation(&status, "debug.log").0, Situation::Ignored);

        run(&s).await;
        let root = s.service.job_status(s.job, "").await.unwrap();
        assert_eq!(root.items.len(), 1);
        assert_eq!(root.items[0].situation, Situation::Synced);
        assert_eq!((root.local_files, root.synced_files), (2, 2));
        assert_eq!(root.synced_bytes, 3);
        assert_eq!(root.counts.synced, 2);
        let status = s.service.job_status(s.job, "Docs").await.unwrap();
        let a = status.items.iter().find(|i| i.name == "a.txt").unwrap();
        // RN-101: data no computador, no destino (guardada no sync) e do último sync.
        assert!(a.local_modified.is_some() && a.remote_modified.is_some());
        assert!(a.synced_at.is_some());
        assert_eq!(status.remote_checked_at, None);

        // Mudou no computador; e um arquivo novo.
        std::fs::write(s.source.join("a.txt"), "aaa").unwrap();
        std::fs::write(s.source.join("novo.txt"), "n").unwrap();
        // Mudou no destino (fora do app).
        std::fs::write(s.dest.join("Docs/sub/b.txt"), "outro").unwrap();
        let status = s.service.job_status(s.job, "Docs").await.unwrap();
        assert_eq!(situation(&status, "a.txt").0, Situation::LocallyChanged);
        assert_eq!(situation(&status, "novo.txt").0, Situation::Pending);
        // Sem consultar o destino, a mudança lá ainda não aparece.
        assert_eq!(situation(&status, "sub").0, Situation::Synced);

        // RN-103: "Atualizar do destino".
        let status = s.service.refresh_job_status(s.job, "Docs").await.unwrap();
        assert!(status.remote_checked_at.is_some());
        assert_eq!(situation(&status, "sub").0, Situation::RemotelyChanged);
        let sub = s.service.job_status(s.job, "Docs/sub").await.unwrap();
        assert_eq!(
            situation(&sub, "b.txt"),
            (Situation::RemotelyChanged, Some("alterado no destino"))
        );
        // Mudou dos dois lados: conflito.
        std::fs::write(s.source.join("sub/b.txt"), "meu").unwrap();
        let sub = s.service.job_status(s.job, "Docs/sub").await.unwrap();
        assert_eq!(situation(&sub, "b.txt").0, Situation::Conflict);

        // Uma nova execução descarta a foto (ela já não vale).
        run(&s).await;
        let status = s.service.job_status(s.job, "Docs").await.unwrap();
        assert_eq!(status.remote_checked_at, None);
    }

    #[tokio::test]
    async fn rn100_resumo_da_tarefa() {
        let s = setup(Delivery::Files);
        run(&s).await;
        let job = s.service.list_jobs().unwrap().remove(0);
        let last = job.last_run.unwrap();
        // 2 arquivos enviados (as pastas não contam como item transferido).
        assert_eq!((last.items, last.bytes), (Some(2), Some(3)));
        let sync = job.sync.unwrap();
        assert_eq!(sync.schedule_state, "active");
        assert_eq!(sync.next_run.map(|t| t.timestamp()), Some(FAKE_NEXT_RUN));
    }

    #[tokio::test]
    async fn compactada_nao_tem_status_por_arquivo() {
        let s = setup(Delivery::Archive {
            encrypt: false,
            format: syncly_core::archive::ArchiveFormat::Zip,
            retention: Retention::default(),
        });
        let status = s.service.job_status(s.job, "").await.unwrap();
        assert!(!status.per_file && status.items.is_empty());
        assert!(s.service.refresh_job_status(s.job, "").await.is_err());
    }
}
