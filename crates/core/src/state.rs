//! `state.db`: o que o app sabe (índice, execuções, backups). SQLite em WAL,
//! seguro para GUI e CLI abertos ao mesmo tempo. Pode ser reconstruído (RN-56).

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use uuid::Uuid;

use crate::domain::run::{HISTORY_MAX_DAYS, HISTORY_MAX_RUNS};
use crate::domain::{EntryKind, IndexEntry, RunKind, RunRecord, RunStatus, RunTrigger};
use crate::error::{CoreError, CoreResult, IoContext};
use crate::fsutil::create_private_file;
use crate::provider::RemoteEntry;
use crate::sync::status::{RemoteInfo, RemoteSnapshot};

/// Migrações sequenciais; a posição no array é a versão (`PRAGMA user_version`).
/// Nunca altere uma migração já publicada — acrescente outra.
const MIGRATIONS: &[&str] = &[
    r"
    CREATE TABLE file_index (
        job_id          TEXT    NOT NULL,
        path            TEXT    NOT NULL,
        kind            TEXT    NOT NULL CHECK (kind IN ('file', 'folder')),
        local_size      INTEGER NOT NULL,
        local_mtime_ns  INTEGER NOT NULL,
        local_hash      TEXT,
        remote_id       TEXT    NOT NULL,
        remote_hash     TEXT,
        synced_at       TEXT    NOT NULL,
        PRIMARY KEY (job_id, path)
    ) WITHOUT ROWID;

    CREATE TABLE runs (
        id           TEXT PRIMARY KEY,
        job_id       TEXT NOT NULL,
        kind         TEXT NOT NULL,
        trigger      TEXT NOT NULL,
        status       TEXT NOT NULL,
        started_at   TEXT NOT NULL,
        finished_at  TEXT,
        stats_json   TEXT,
        error        TEXT
    );
    CREATE INDEX runs_by_job ON runs (job_id, started_at DESC);

    CREATE TABLE backups (
        id          TEXT PRIMARY KEY,
        job_id      TEXT    NOT NULL,
        location    TEXT    NOT NULL CHECK (location IN ('local', 'remote')),
        locator     TEXT    NOT NULL,
        size        INTEGER NOT NULL,
        sha256      TEXT    NOT NULL,
        key_id      TEXT,
        created_at  TEXT    NOT NULL,
        run_id      TEXT
    );
    CREATE INDEX backups_by_job ON backups (job_id, created_at DESC);

    CREATE TABLE account_state (
        account_id  TEXT PRIMARY KEY,
        status      TEXT NOT NULL,
        cursor      TEXT,
        updated_at  TEXT NOT NULL
    );
",
    // 2: versões de tarefas compactadas (RN-50 a RN-55, RN-59). A tabela `backups`
    // da migração 1 nunca foi usada.
    r"
    DROP TABLE IF EXISTS backups;
    CREATE TABLE archives (
        id           TEXT    PRIMARY KEY,
        job_id       TEXT    NOT NULL,
        account_id   TEXT    NOT NULL,
        dest_path    TEXT    NOT NULL,
        remote_id    TEXT    NOT NULL,
        name         TEXT    NOT NULL,
        format       TEXT    NOT NULL,
        size         INTEGER NOT NULL,
        hash         TEXT,
        fingerprint  TEXT    NOT NULL,
        created_at   TEXT    NOT NULL
    );
    CREATE INDEX archives_by_destination ON archives (job_id, account_id, dest_path, created_at DESC);
",
    // 3: versões recebidas por tarefas que recebem (RN-107).
    r"
    CREATE TABLE received (
        id          TEXT    PRIMARY KEY,
        job_id      TEXT    NOT NULL,
        remote_id   TEXT    NOT NULL,
        name        TEXT    NOT NULL,
        size        INTEGER NOT NULL,
        hash        TEXT,
        dest        TEXT    NOT NULL,
        received_at TEXT    NOT NULL
    );
    CREATE INDEX received_by_job ON received (job_id, received_at DESC);
",
    // 4: status (RN-101, RN-103) — modificação no destino guardada no sync e a "foto" do
    // destino tirada por "Atualizar do destino".
    r"
    ALTER TABLE file_index ADD COLUMN remote_modified_at TEXT;
    ALTER TABLE file_index ADD COLUMN remote_modified_by TEXT;
    CREATE TABLE remote_snapshot (
        job_id      TEXT    NOT NULL,
        path        TEXT    NOT NULL,
        kind        TEXT    NOT NULL,
        size        INTEGER,
        hash        TEXT,
        modified_at TEXT,
        modified_by TEXT,
        PRIMARY KEY (job_id, path)
    );
    CREATE TABLE remote_snapshot_meta (
        job_id     TEXT PRIMARY KEY,
        fetched_at TEXT NOT NULL
    );
",
    // 5: contas que precisam ser reconectadas (RN-06).
    r"
    CREATE TABLE account_status (
        account_id            TEXT PRIMARY KEY,
        needs_reconnect_since TEXT NOT NULL
    );
",
];

pub struct StateDb {
    conn: Connection,
}

/// Uma versão criada por uma tarefa compactada (RN-50). A retenção só considera o que
/// está registrado aqui — nunca arquivos que a tarefa não criou (RN-55).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveRecord {
    pub id: Uuid,
    pub job_id: Uuid,
    pub account_id: Uuid,
    pub dest_path: String,
    pub remote_id: String,
    pub name: String,
    pub format: String,
    pub size: u64,
    pub hash: Option<String>,
    /// RN-59: resumo do conteúdo das fontes (caminhos, tamanhos, datas).
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

/// Uma versão recebida por uma tarefa que recebe (RN-107).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedRecord {
    pub id: Uuid,
    pub job_id: Uuid,
    pub remote_id: String,
    pub name: String,
    pub size: u64,
    pub hash: Option<String>,
    /// Pasta criada no computador.
    pub dest: String,
    pub received_at: DateTime<Utc>,
}

impl ReceivedRecord {
    /// RN-107: é a mesma versão (mesmo item, nome e tamanho; e conteúdo, se houver hash)?
    pub fn is_same(&self, e: &RemoteEntry) -> bool {
        self.remote_id == e.id
            && self.name == e.name
            && e.size.is_none_or(|s| s == self.size)
            && (self.hash.is_none() || e.hash.is_none() || self.hash == e.hash)
    }
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn parse_ts(s: &str) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
        })
}

const ARCHIVE_COLUMNS: &str = "id, job_id, account_id, dest_path, remote_id, name, format, size, \
     hash, fingerprint, created_at";

fn archive_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ArchiveRecord> {
    let id: String = r.get(0)?;
    let job: String = r.get(1)?;
    let account: String = r.get(2)?;
    let created: String = r.get(10)?;
    Ok(ArchiveRecord {
        id: id.parse().map_err(|_| bad_value("id", &id))?,
        job_id: job.parse().map_err(|_| bad_value("tarefa", &job))?,
        account_id: account.parse().map_err(|_| bad_value("conta", &account))?,
        dest_path: r.get(3)?,
        remote_id: r.get(4)?,
        name: r.get(5)?,
        format: r.get(6)?,
        size: u64::try_from(r.get::<_, i64>(7)?).unwrap_or(0),
        hash: r.get(8)?,
        fingerprint: r.get(9)?,
        created_at: parse_ts(&created)?,
    })
}

fn bad_value(what: &str, value: &str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        format!("{what} inválido no banco: {value}").into(),
    )
}

impl StateDb {
    pub fn open(path: &Path) -> CoreResult<Self> {
        // SEG-53: cria o arquivo já com 0600 antes de o SQLite abri-lo.
        if !path.exists() {
            match create_private_file(path) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e).at(path),
            }
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> CoreResult<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> CoreResult<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut db = Self { conn };
        db.migrate()?;
        Ok(db)
    }

    fn migrate(&mut self) -> CoreResult<()> {
        let current: i64 = self
            .conn
            .pragma_query_value(None, "user_version", |r| r.get(0))?;
        let current = usize::try_from(current).unwrap_or(usize::MAX);
        if current > MIGRATIONS.len() {
            return Err(CoreError::Config(format!(
                "banco de estado versão {current} é mais novo que este app"
            )));
        }
        for (i, sql) in MIGRATIONS.iter().enumerate().skip(current) {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(
                None,
                "user_version",
                i64::try_from(i + 1).unwrap_or(i64::MAX),
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    // ---- Índice ----

    pub fn load_index(&self, job_id: Uuid) -> CoreResult<BTreeMap<String, IndexEntry>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, kind, local_size, local_mtime_ns, local_hash, remote_id, remote_hash,
                    synced_at, remote_modified_at, remote_modified_by
             FROM file_index WHERE job_id = ?1",
        )?;
        let rows = stmt.query_map(params![job_id.to_string()], |r| {
            let kind: String = r.get(1)?;
            let synced_at: String = r.get(7)?;
            let remote_modified_at: Option<String> = r.get(8)?;
            Ok(IndexEntry {
                path: r.get(0)?,
                kind: EntryKind::parse(&kind).ok_or_else(|| bad_value("tipo", &kind))?,
                local_size: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                local_mtime_ns: r.get(3)?,
                local_hash: r.get(4)?,
                remote_id: r.get(5)?,
                remote_hash: r.get(6)?,
                synced_at: parse_ts(&synced_at)?,
                remote_modified_at: remote_modified_at.as_deref().map(parse_ts).transpose()?,
                remote_modified_by: r.get(9)?,
            })
        })?;
        let mut out = BTreeMap::new();
        for row in rows {
            let entry = row?;
            out.insert(entry.path.clone(), entry);
        }
        Ok(out)
    }

    /// Grava várias entradas numa transação (RN-38: o executor chama a cada lote confirmado).
    pub fn upsert_index(&mut self, job_id: Uuid, entries: &[IndexEntry]) -> CoreResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO file_index
                   (job_id, path, kind, local_size, local_mtime_ns, local_hash, remote_id, remote_hash,
                    synced_at, remote_modified_at, remote_modified_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT (job_id, path) DO UPDATE SET
                   kind = excluded.kind, local_size = excluded.local_size,
                   local_mtime_ns = excluded.local_mtime_ns, local_hash = excluded.local_hash,
                   remote_id = excluded.remote_id, remote_hash = excluded.remote_hash,
                   synced_at = excluded.synced_at,
                   remote_modified_at = excluded.remote_modified_at,
                   remote_modified_by = excluded.remote_modified_by",
            )?;
            let job = job_id.to_string();
            for e in entries {
                stmt.execute(params![
                    job,
                    e.path,
                    e.kind.as_str(),
                    i64::try_from(e.local_size).unwrap_or(i64::MAX),
                    e.local_mtime_ns,
                    e.local_hash,
                    e.remote_id,
                    e.remote_hash,
                    ts(e.synced_at),
                    e.remote_modified_at.map(ts),
                    e.remote_modified_by,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn remove_index(&mut self, job_id: Uuid, paths: &[String]) -> CoreResult<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt =
                tx.prepare_cached("DELETE FROM file_index WHERE job_id = ?1 AND path = ?2")?;
            let job = job_id.to_string();
            for p in paths {
                stmt.execute(params![job, p])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Esquece o índice e as versões registradas. Os arquivos no destino ficam como estão.
    pub fn clear_job(&mut self, job_id: Uuid) -> CoreResult<()> {
        let tx = self.conn.transaction()?;
        for table in [
            "file_index",
            "archives",
            "received",
            "remote_snapshot",
            "remote_snapshot_meta",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE job_id = ?1"),
                params![job_id.to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- Execuções ----

    pub fn start_run(&self, job_id: Uuid, kind: RunKind, trigger: RunTrigger) -> CoreResult<Uuid> {
        let id = Uuid::new_v4();
        self.conn.execute(
            "INSERT INTO runs (id, job_id, kind, trigger, status, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id.to_string(),
                job_id.to_string(),
                kind.as_str(),
                trigger.as_str(),
                RunStatus::Running.as_str(),
                ts(Utc::now()),
            ],
        )?;
        Ok(id)
    }

    pub fn finish_run(
        &self,
        run_id: Uuid,
        status: RunStatus,
        stats: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> CoreResult<()> {
        self.conn.execute(
            "UPDATE runs SET status = ?2, finished_at = ?3, stats_json = ?4, error = ?5 WHERE id = ?1",
            params![
                run_id.to_string(),
                status.as_str(),
                ts(Utc::now()),
                stats.map(ToString::to_string),
                error,
            ],
        )?;
        Ok(())
    }

    /// RN-36: já houve alguma execução bem-sucedida deste tipo?
    pub fn has_successful_run(&self, job_id: Uuid, kind: RunKind) -> CoreResult<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM runs WHERE job_id = ?1 AND kind = ?2 AND status = ?3)",
            params![
                job_id.to_string(),
                kind.as_str(),
                RunStatus::Success.as_str()
            ],
            |r| r.get(0),
        )?)
    }

    // ---- Versões de tarefas compactadas ----

    pub fn insert_archive(&self, r: &ArchiveRecord) -> CoreResult<()> {
        self.conn.execute(
            "INSERT INTO archives
               (id, job_id, account_id, dest_path, remote_id, name, format, size, hash, fingerprint, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                r.id.to_string(),
                r.job_id.to_string(),
                r.account_id.to_string(),
                r.dest_path,
                r.remote_id,
                r.name,
                r.format,
                i64::try_from(r.size).unwrap_or(i64::MAX),
                r.hash,
                r.fingerprint,
                ts(r.created_at),
            ],
        )?;
        Ok(())
    }

    /// Versões de uma tarefa num destino, da mais nova para a mais antiga.
    pub fn list_archives(
        &self,
        job_id: Uuid,
        account_id: Uuid,
        dest_path: &str,
    ) -> CoreResult<Vec<ArchiveRecord>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {ARCHIVE_COLUMNS} FROM archives
             WHERE job_id = ?1 AND account_id = ?2 AND dest_path = ?3
             ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map(
            params![job_id.to_string(), account_id.to_string(), dest_path],
            archive_row,
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Todas as versões de uma tarefa, em qualquer destino, da mais nova para a mais antiga.
    pub fn list_job_archives(&self, job_id: Uuid) -> CoreResult<Vec<ArchiveRecord>> {
        let mut stmt = self.conn.prepare_cached(&format!(
            "SELECT {ARCHIVE_COLUMNS} FROM archives WHERE job_id = ?1 ORDER BY created_at DESC"
        ))?;
        let rows = stmt.query_map(params![job_id.to_string()], archive_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ---- Contas (RN-06) ----

    /// Marca a conta como "requer reconexão"; mantém a data da primeira vez.
    pub fn mark_needs_reconnect(&self, account_id: Uuid, at: DateTime<Utc>) -> CoreResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO account_status (account_id, needs_reconnect_since)
             VALUES (?1, ?2)",
            params![account_id.to_string(), ts(at)],
        )?;
        Ok(())
    }

    pub fn clear_needs_reconnect(&self, account_id: Uuid) -> CoreResult<()> {
        self.conn.execute(
            "DELETE FROM account_status WHERE account_id = ?1",
            params![account_id.to_string()],
        )?;
        Ok(())
    }

    /// Contas que precisam ser reconectadas → desde quando.
    pub fn accounts_needing_reconnect(&self) -> CoreResult<HashMap<Uuid, DateTime<Utc>>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT account_id, needs_reconnect_since FROM account_status")?;
        let rows = stmt.query_map([], |r| {
            let id: String = r.get(0)?;
            let since: String = r.get(1)?;
            Ok((
                id.parse().map_err(|_| bad_value("conta", &id))?,
                parse_ts(&since)?,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    // ---- Foto do destino (RN-103) ----

    /// Substitui a foto do destino de uma tarefa.
    pub fn save_snapshot(&mut self, job_id: Uuid, snapshot: &RemoteSnapshot) -> CoreResult<()> {
        let job = job_id.to_string();
        let tx = self.conn.transaction()?;
        tx.execute(
            "DELETE FROM remote_snapshot WHERE job_id = ?1",
            params![job],
        )?;
        {
            let mut stmt = tx.prepare_cached(
                "INSERT INTO remote_snapshot (job_id, path, kind, size, hash, modified_at, modified_by)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (path, e) in &snapshot.entries {
                stmt.execute(params![
                    job,
                    path,
                    e.kind.as_str(),
                    e.size.map(|s| i64::try_from(s).unwrap_or(i64::MAX)),
                    e.hash,
                    e.modified_at.map(ts),
                    e.modified_by,
                ])?;
            }
        }
        tx.execute(
            "INSERT INTO remote_snapshot_meta (job_id, fetched_at) VALUES (?1, ?2)
             ON CONFLICT (job_id) DO UPDATE SET fetched_at = excluded.fetched_at",
            params![job, ts(snapshot.fetched_at)],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn load_snapshot(&self, job_id: Uuid) -> CoreResult<Option<RemoteSnapshot>> {
        let job = job_id.to_string();
        let fetched: Option<String> = self
            .conn
            .query_row(
                "SELECT fetched_at FROM remote_snapshot_meta WHERE job_id = ?1",
                params![job],
                |r| r.get(0),
            )
            .optional()?;
        let Some(fetched) = fetched else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare_cached(
            "SELECT path, kind, size, hash, modified_at, modified_by
             FROM remote_snapshot WHERE job_id = ?1",
        )?;
        let rows = stmt.query_map(params![job], |r| {
            let kind: String = r.get(1)?;
            let modified: Option<String> = r.get(4)?;
            Ok((
                r.get::<_, String>(0)?,
                RemoteInfo {
                    kind: EntryKind::parse(&kind).ok_or_else(|| bad_value("tipo", &kind))?,
                    size: r
                        .get::<_, Option<i64>>(2)?
                        .map(|s| u64::try_from(s).unwrap_or(0)),
                    hash: r.get(3)?,
                    modified_at: modified.as_deref().map(parse_ts).transpose()?,
                    modified_by: r.get(5)?,
                },
            ))
        })?;
        Ok(Some(RemoteSnapshot {
            fetched_at: parse_ts(&fetched)?,
            entries: rows.collect::<Result<_, _>>()?,
        }))
    }

    /// Uma execução muda o destino: a foto antiga deixa de valer.
    pub fn clear_snapshot(&self, job_id: Uuid) -> CoreResult<()> {
        let job = job_id.to_string();
        self.conn.execute(
            "DELETE FROM remote_snapshot WHERE job_id = ?1",
            params![job],
        )?;
        self.conn.execute(
            "DELETE FROM remote_snapshot_meta WHERE job_id = ?1",
            params![job],
        )?;
        Ok(())
    }

    // ---- Versões recebidas ----

    pub fn insert_received(&self, r: &ReceivedRecord) -> CoreResult<()> {
        self.conn.execute(
            "INSERT INTO received (id, job_id, remote_id, name, size, hash, dest, received_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                r.id.to_string(),
                r.job_id.to_string(),
                r.remote_id,
                r.name,
                i64::try_from(r.size).unwrap_or(i64::MAX),
                r.hash,
                r.dest,
                ts(r.received_at),
            ],
        )?;
        Ok(())
    }

    /// A última versão recebida pela tarefa.
    pub fn last_received(&self, job_id: Uuid) -> CoreResult<Option<ReceivedRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, job_id, remote_id, name, size, hash, dest, received_at FROM received
             WHERE job_id = ?1 ORDER BY received_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![job_id.to_string()], |r| {
            let id: String = r.get(0)?;
            let job: String = r.get(1)?;
            let at: String = r.get(7)?;
            Ok(ReceivedRecord {
                id: id.parse().map_err(|_| bad_value("id", &id))?,
                job_id: job.parse().map_err(|_| bad_value("tarefa", &job))?,
                remote_id: r.get(2)?,
                name: r.get(3)?,
                size: u64::try_from(r.get::<_, i64>(4)?).unwrap_or(0),
                hash: r.get(5)?,
                dest: r.get(6)?,
                received_at: parse_ts(&at)?,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn delete_archive(&self, id: Uuid) -> CoreResult<()> {
        self.conn.execute(
            "DELETE FROM archives WHERE id = ?1",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// A última execução de verdade — um disparo descartado (RN-90) não esconde a execução
    /// que estava rodando.
    pub fn last_run(&self, job_id: Uuid) -> CoreResult<Option<RunRecord>> {
        Ok(self
            .recent_runs(job_id, 20)?
            .into_iter()
            .find(|r| r.status != RunStatus::Discarded))
    }

    /// RN-84: quantas das últimas execuções seguidas foram adiadas (disparos descartados,
    /// RN-90, não interrompem a sequência). Conta até [`timers::MAX_RETRIES`] + 1 — o
    /// suficiente para decidir.
    ///
    /// [`timers::MAX_RETRIES`]: crate::timers::MAX_RETRIES
    pub fn postponed_streak(&self, job_id: Uuid) -> CoreResult<u32> {
        let limit = crate::timers::MAX_RETRIES * 2 + 1;
        let streak = self
            .recent_runs(job_id, limit)?
            .into_iter()
            .filter(|r| r.status != RunStatus::Discarded)
            .take_while(|r| r.status == RunStatus::Postponed)
            .count();
        Ok(u32::try_from(streak).unwrap_or(u32::MAX))
    }

    /// RN-93: apaga execuções com mais de `HISTORY_MAX_DAYS` dias e, por tarefa, as que
    /// passarem de `HISTORY_MAX_RUNS`. Retorna quantas saíram.
    pub fn prune_history(&self, now: DateTime<Utc>) -> CoreResult<usize> {
        let cutoff = now - chrono::Duration::days(HISTORY_MAX_DAYS);
        let old = self.conn.execute(
            "DELETE FROM runs WHERE started_at < ?1",
            params![ts(cutoff)],
        )?;
        let extra = self.conn.execute(
            "DELETE FROM runs WHERE id IN (
                SELECT id FROM (
                    SELECT id, ROW_NUMBER() OVER (PARTITION BY job_id ORDER BY started_at DESC) AS n
                    FROM runs
                ) WHERE n > ?1
            )",
            params![HISTORY_MAX_RUNS],
        )?;
        Ok(old + extra)
    }

    pub fn recent_runs(&self, job_id: Uuid, limit: u32) -> CoreResult<Vec<RunRecord>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, job_id, kind, trigger, status, started_at, finished_at, stats_json, error
             FROM runs WHERE job_id = ?1 ORDER BY started_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![job_id.to_string(), limit], |r| {
            let id: String = r.get(0)?;
            let job: String = r.get(1)?;
            let kind: String = r.get(2)?;
            let trigger: String = r.get(3)?;
            let status: String = r.get(4)?;
            let started: String = r.get(5)?;
            let finished: Option<String> = r.get(6)?;
            let stats: Option<String> = r.get(7)?;
            Ok(RunRecord {
                id: id.parse().map_err(|_| bad_value("id", &id))?,
                job_id: job.parse().map_err(|_| bad_value("tarefa", &job))?,
                kind: RunKind::parse(&kind).ok_or_else(|| bad_value("tipo", &kind))?,
                trigger: RunTrigger::parse(&trigger)
                    .ok_or_else(|| bad_value("gatilho", &trigger))?,
                status: RunStatus::parse(&status).ok_or_else(|| bad_value("status", &status))?,
                started_at: parse_ts(&started)?,
                finished_at: finished.as_deref().map(parse_ts).transpose()?,
                stats: stats.and_then(|s| serde_json::from_str(&s).ok()),
                error: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str) -> IndexEntry {
        IndexEntry {
            path: path.into(),
            kind: EntryKind::File,
            local_size: 3,
            local_mtime_ns: 1_700_000_000_000_000_000,
            local_hash: Some("h".into()),
            remote_id: format!("id-{path}"),
            remote_hash: Some("h".into()),
            synced_at: Utc::now(),
            remote_modified_at: None,
            remote_modified_by: None,
        }
    }

    #[test]
    fn indice_ida_e_volta() {
        let mut db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        db.upsert_index(job, &[entry("a/x.txt"), entry("a/y.txt")])
            .unwrap();
        let mut changed = entry("a/x.txt");
        changed.local_size = 10;
        db.upsert_index(job, &[changed]).unwrap();
        db.remove_index(job, &["a/y.txt".into()]).unwrap();

        let index = db.load_index(job).unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index["a/x.txt"].local_size, 10);
        assert!(db.load_index(Uuid::new_v4()).unwrap().is_empty());
    }

    #[test]
    fn execucoes() {
        let db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        let run = db
            .start_run(job, RunKind::Sync, RunTrigger::Manual)
            .unwrap();
        db.finish_run(
            run,
            RunStatus::Success,
            Some(&serde_json::json!({"uploads": 2})),
            None,
        )
        .unwrap();
        let last = db.last_run(job).unwrap().unwrap();
        assert_eq!(last.status, RunStatus::Success);
        assert_eq!(last.stats.unwrap()["uploads"], 2);
    }

    #[test]
    fn versoes_de_tarefa_compactada() {
        let db = StateDb::open_in_memory().unwrap();
        let (job, account) = (Uuid::new_v4(), Uuid::new_v4());
        let record = |name: &str, minutes: i64| ArchiveRecord {
            id: Uuid::new_v4(),
            job_id: job,
            account_id: account,
            dest_path: "Backups".into(),
            remote_id: format!("id-{name}"),
            name: name.into(),
            format: "tar_zst".into(),
            size: 10,
            hash: Some("h".into()),
            fingerprint: "f".into(),
            created_at: Utc::now() - chrono::Duration::minutes(minutes),
        };
        let old = record("velha", 60);
        db.insert_archive(&old).unwrap();
        db.insert_archive(&record("nova", 1)).unwrap();
        let listed = db.list_archives(job, account, "Backups").unwrap();
        assert_eq!(listed[0].name, "nova");
        assert_eq!(listed.len(), 2);
        // Outro destino não se mistura.
        assert!(db.list_archives(job, account, "Outra").unwrap().is_empty());
        assert_eq!(db.list_job_archives(job).unwrap().len(), 2);
        assert!(db.list_job_archives(Uuid::new_v4()).unwrap().is_empty());
        db.delete_archive(old.id).unwrap();
        assert_eq!(db.list_archives(job, account, "Backups").unwrap().len(), 1);
    }

    /// Execução fictícia iniciada `days_ago` dias atrás.
    fn insert_run(db: &StateDb, job: Uuid, days_ago: i64, status: RunStatus) {
        let at = Utc::now() - chrono::Duration::days(days_ago);
        db.conn
            .execute(
                "INSERT INTO runs (id, job_id, kind, trigger, status, started_at, finished_at)
                 VALUES (?1, ?2, 'sync', 'scheduled', ?3, ?4, ?4)",
                params![
                    Uuid::new_v4().to_string(),
                    job.to_string(),
                    status.as_str(),
                    ts(at)
                ],
            )
            .unwrap();
    }

    #[test]
    fn rn93_historico_limitado_por_idade_e_quantidade() {
        let db = StateDb::open_in_memory().unwrap();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        insert_run(&db, a, 200, RunStatus::Success);
        insert_run(&db, a, 10, RunStatus::Success);
        for _ in 0..(HISTORY_MAX_RUNS + 5) {
            insert_run(&db, b, 1, RunStatus::Success);
        }
        let removed = db.prune_history(Utc::now()).unwrap();
        assert_eq!(removed, 1 + 5);
        assert_eq!(db.recent_runs(a, 5000).unwrap().len(), 1);
        assert_eq!(
            db.recent_runs(b, 5000).unwrap().len(),
            usize::try_from(HISTORY_MAX_RUNS).unwrap()
        );
        // Nada mais a apagar.
        assert_eq!(db.prune_history(Utc::now()).unwrap(), 0);
    }

    #[test]
    fn rn90_disparo_descartado_nao_vira_a_ultima_execucao() {
        let db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        insert_run(&db, job, 2, RunStatus::Success);
        insert_run(&db, job, 1, RunStatus::Discarded);
        assert_eq!(
            db.last_run(job).unwrap().unwrap().status,
            RunStatus::Success
        );
        assert_eq!(
            db.recent_runs(job, 5).unwrap()[0].status,
            RunStatus::Discarded
        );
    }

    #[test]
    fn rn84_adiamentos_seguidos() {
        let db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        assert_eq!(db.postponed_streak(job).unwrap(), 0);
        insert_run(&db, job, 5, RunStatus::Postponed);
        insert_run(&db, job, 4, RunStatus::Success);
        insert_run(&db, job, 3, RunStatus::Postponed);
        insert_run(&db, job, 2, RunStatus::Discarded);
        insert_run(&db, job, 1, RunStatus::Postponed);
        // O sucesso no meio zera a sequência; o descartado não a interrompe.
        assert_eq!(db.postponed_streak(job).unwrap(), 2);
        insert_run(&db, job, 0, RunStatus::Failed);
        assert_eq!(db.postponed_streak(job).unwrap(), 0);
    }

    #[test]
    fn rn06_conta_que_precisa_ser_reconectada() {
        let db = StateDb::open_in_memory().unwrap();
        let account = Uuid::new_v4();
        assert!(db.accounts_needing_reconnect().unwrap().is_empty());
        let first = Utc::now() - chrono::Duration::hours(2);
        db.mark_needs_reconnect(account, first).unwrap();
        // Marcar de novo não muda a data da primeira vez.
        db.mark_needs_reconnect(account, Utc::now()).unwrap();
        let marked = db.accounts_needing_reconnect().unwrap();
        assert_eq!(marked[&account].timestamp(), first.timestamp());
        db.clear_needs_reconnect(account).unwrap();
        assert!(db.accounts_needing_reconnect().unwrap().is_empty());
    }

    #[test]
    fn rn103_foto_do_destino() {
        let mut db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        assert_eq!(db.load_snapshot(job).unwrap(), None);
        let mut entries = BTreeMap::new();
        entries.insert(
            "Docs/a.txt".to_string(),
            RemoteInfo {
                kind: EntryKind::File,
                size: Some(3),
                hash: Some("h".into()),
                modified_at: DateTime::from_timestamp(1_700_000_000, 0),
                modified_by: Some("Ana".into()),
            },
        );
        let snapshot = RemoteSnapshot {
            fetched_at: Utc::now(),
            entries,
        };
        db.save_snapshot(job, &snapshot).unwrap();
        let loaded = db.load_snapshot(job).unwrap().unwrap();
        assert_eq!(loaded.entries, snapshot.entries);
        // Substitui por inteiro.
        db.save_snapshot(
            job,
            &RemoteSnapshot {
                fetched_at: Utc::now(),
                entries: BTreeMap::new(),
            },
        )
        .unwrap();
        assert!(db.load_snapshot(job).unwrap().unwrap().entries.is_empty());
        db.clear_snapshot(job).unwrap();
        assert_eq!(db.load_snapshot(job).unwrap(), None);
    }

    #[test]
    fn rn107_ultima_versao_recebida() {
        let mut db = StateDb::open_in_memory().unwrap();
        let job = Uuid::new_v4();
        assert_eq!(db.last_received(job).unwrap(), None);
        let record = |name: &str, minutes: i64| ReceivedRecord {
            id: Uuid::new_v4(),
            job_id: job,
            remote_id: format!("id-{name}"),
            name: name.into(),
            size: 10,
            hash: Some("h".into()),
            dest: format!("/r/{name}"),
            received_at: Utc::now() - chrono::Duration::minutes(minutes),
        };
        db.insert_received(&record("velha", 60)).unwrap();
        db.insert_received(&record("nova", 1)).unwrap();
        let last = db.last_received(job).unwrap().unwrap();
        assert_eq!(last.name, "nova");

        let mut entry = RemoteEntry {
            id: "id-nova".into(),
            name: "nova".into(),
            kind: EntryKind::File,
            size: Some(10),
            hash: Some("h".into()),
            modified_at: None,
            modified_by: None,
        };
        assert!(last.is_same(&entry));
        // Substituída no destino com o mesmo nome: conteúdo diferente, recebe de novo.
        entry.hash = Some("outro".into());
        assert!(!last.is_same(&entry));

        db.clear_job(job).unwrap();
        assert_eq!(db.last_received(job).unwrap(), None);
    }

    #[test]
    fn arquivo_criado_com_migracoes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        drop(StateDb::open(&path).unwrap());
        // Reabrir não reaplica migrações.
        let db = StateDb::open(&path).unwrap();
        assert!(db.load_index(Uuid::new_v4()).unwrap().is_empty());
    }
}
