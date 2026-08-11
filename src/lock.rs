use anyhow::anyhow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(crate) struct FileLock {
    path: PathBuf,
}

impl FileLock {
    pub(crate) fn acquire(dir: &Path, timeout: Duration) -> anyhow::Result<Self> {
        let lock_path = dir.join(".jardedupli.lock");
        let deadline = Instant::now() + timeout;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut f) => {
                    use std::io::Write;
                    let _ = writeln!(f, "pid={}", std::process::id());
                    return Ok(FileLock { path: lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if Instant::now() >= deadline {
                        return Err(anyhow!(
                            "lock timeout after {}s (another instance running? stale lock at {})",
                            timeout.as_secs(),
                            lock_path.display()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
