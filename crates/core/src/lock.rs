//! Trava exclusiva por tarefa (RN-90): GUI e timer nunca rodam a mesma tarefa juntos.
//! Usa `flock` via `File::try_lock`; o SO libera a trava se o processo morrer.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use uuid::Uuid;

use crate::error::{CoreError, CoreResult, IoContext};

#[derive(Debug)]
pub struct JobLock {
    _file: File,
}

impl JobLock {
    /// Tenta adquirir sem esperar. `Err(CoreError::JobLocked)` se já houver execução.
    pub fn try_acquire(lock_dir: &Path, job_id: Uuid) -> CoreResult<Self> {
        let path = lock_dir.join(format!("job-{job_id}.lock"));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .at(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file }),
            Err(TryLockError::WouldBlock) => Err(CoreError::JobLocked),
            Err(TryLockError::Error(e)) => Err(e).at(&path),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segunda_trava_da_mesma_tarefa_falha() {
        let dir = tempfile::tempdir().unwrap();
        let job = Uuid::new_v4();
        let first = JobLock::try_acquire(dir.path(), job).unwrap();
        assert!(matches!(
            JobLock::try_acquire(dir.path(), job),
            Err(CoreError::JobLocked)
        ));
        // Outra tarefa não é afetada.
        JobLock::try_acquire(dir.path(), Uuid::new_v4()).unwrap();
        drop(first);
        JobLock::try_acquire(dir.path(), job).unwrap();
    }
}
