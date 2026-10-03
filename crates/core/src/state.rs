//! `state.db`: o que o app sabe (índice, execuções, backups). SQLite em WAL,
//! seguro para GUI e CLI abertos ao mesmo tempo. Pode ser reconstruído (RN-56).

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, params};
use uuid::Uuid;

use crate::domain::{EntryKind, IndexEntry, RunKind, RunRecord, RunStatus, RunTrigger};
use crate::error::{CoreError, CoreResult, IoContext};
use crate::fsutil::create_private_file;

/// Migrações sequenciais; a posição no array é a versão (`PRAGMA user_version`).
/// Nunca altere uma migração já publicada — acrescente outra.
const MIGRATIONS: &[&str] = &[r"
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
"];

pub struct StateDb {
    conn: Connection,
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
            "SELECT path, kind, local_size, local_mtime_ns, local_hash, remote_id, remote_hash, synced_at
             FROM file_index WHERE job_id = ?1",
        )?;
        let rows = stmt.query_map(params![job_id.to_string()], |r| {
            let kind: String = r.get(1)?;
            let synced_at: String = r.get(7)?;
            Ok(IndexEntry {
                path: r.get(0)?,
                kind: EntryKind::parse(&kind).ok_or_else(|| bad_value("tipo", &kind))?,
                local_size: u64::try_from(r.get::<_, i64>(2)?).unwrap_or(0),
                local_mtime_ns: r.get(3)?,
                local_hash: r.get(4)?,
                remote_id: r.get(5)?,
                remote_hash: r.get(6)?,
                synced_at: parse_ts(&synced_at)?,
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
                   (job_id, path, kind, local_size, local_mtime_ns, local_hash, remote_id, remote_hash, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT (job_id, path) DO UPDATE SET
                   kind = excluded.kind, local_size = excluded.local_size,
                   local_mtime_ns = excluded.local_mtime_ns, local_hash = excluded.local_hash,
                   remote_id = excluded.remote_id, remote_hash = excluded.remote_hash,
                   synced_at = excluded.synced_at",
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

    pub fn clear_job(&mut self, job_id: Uuid) -> CoreResult<()> {
        self.conn.execute(
            "DELETE FROM file_index WHERE job_id = ?1",
            params![job_id.to_string()],
        )?;
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

    pub fn last_run(&self, job_id: Uuid) -> CoreResult<Option<RunRecord>> {
        Ok(self.recent_runs(job_id, 1)?.into_iter().next())
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
    fn arquivo_criado_com_migracoes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.db");
        drop(StateDb::open(&path).unwrap());
        // Reabrir não reaplica migrações.
        let db = StateDb::open(&path).unwrap();
        assert!(db.load_index(Uuid::new_v4()).unwrap().is_empty());
    }
}
