//! Daemon lifecycle helpers such as PID file and shutdown signal handling.

use std::path::PathBuf;
#[cfg(not(test))]
use tokio::signal;
use tracing::info;

/// PID file management
pub struct PidFile {
    /// Filesystem path for the PID file.
    path: PathBuf,
}

impl PidFile {
    /// Creates a PID file manager for a concrete path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the default PID file path under `~/.openpista/`.
    pub fn default_path() -> PathBuf {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".openpista").join("openpista.pid")
    }

    /// Writes the current process ID to the PID file.
    pub async fn write(&self) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let pid = std::process::id().to_string();
        tokio::fs::write(&self.path, pid).await?;
        info!("PID file written: {}", self.path.display());
        Ok(())
    }

    /// Removes the PID file if it exists.
    pub async fn remove(&self) {
        if self.path.exists() {
            let _ = tokio::fs::remove_file(&self.path).await;
            info!("PID file removed: {}", self.path.display());
        }
    }
}

/// Wait for SIGTERM or SIGINT shutdown signal
#[cfg(not(test))]
pub async fn wait_for_shutdown() {
    #[cfg(unix)]
    {
        use signal::unix::{SignalKind, signal};
        let mut sigterm =
            signal(SignalKind::terminate()).expect("Failed to register SIGTERM handler");
        tokio::select! {
            _ = signal::ctrl_c() => {
                info!("Received SIGINT, shutting down");
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down");
            }
        }
    }

    #[cfg(not(unix))]
    {
        signal::ctrl_c().await.expect("Failed to listen for ctrl-c");
        info!("Received Ctrl-C, shutting down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_path_points_to_openpista_pid() {
        let path = PidFile::default_path();
        let text = path.to_string_lossy();
        assert!(text.contains(".openpista"));
        assert!(text.ends_with("openpista.pid"));
    }

    #[tokio::test]
    async fn write_and_remove_pid_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let pid_path = tmp.path().join("nested/openpista.pid");
        let pid_file = PidFile::new(&pid_path);

        pid_file.write().await.expect("pid write");
        assert!(pid_path.exists());

        let written = tokio::fs::read_to_string(&pid_path)
            .await
            .expect("read pid");
        let parsed_pid = written.parse::<u32>().expect("pid should be numeric");
        assert_eq!(parsed_pid, std::process::id());

        pid_file.remove().await;
        assert!(!pid_path.exists());
    }
}
