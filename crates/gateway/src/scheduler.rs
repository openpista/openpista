//! Cron-based scheduler wrapper that emits channel events.

use proto::{ChannelEvent, ChannelId, SessionId};
use tokio::sync::mpsc;
use tokio_cron_scheduler::{Job, JobScheduler};
use tracing::{error, info};

/// Wraps tokio-cron-scheduler to fire ChannelEvents on a schedule
pub struct CronScheduler {
    sched: JobScheduler,
}

impl CronScheduler {
    /// Creates a new scheduler instance.
    pub async fn new() -> Result<Self, String> {
        let sched = JobScheduler::new().await.map_err(|e| e.to_string())?;
        Ok(Self { sched })
    }

    /// Add a cron job that sends a message on the given channel/session
    pub async fn add_job(
        &self,
        cron_expr: &str,
        channel_id: ChannelId,
        session_id: SessionId,
        message: String,
        tx: mpsc::Sender<ChannelEvent>,
    ) -> Result<uuid::Uuid, String> {
        let job = Job::new_async(cron_expr, move |_uuid, _lock| {
            let channel_id = channel_id.clone();
            let session_id = session_id.clone();
            let message = message.clone();
            let tx = tx.clone();
            Box::pin(async move {
                let event = ChannelEvent::new(channel_id, session_id, message);
                if let Err(e) = tx.send(event).await {
                    error!("Cron job failed to send event: {e}");
                }
            })
        })
        .map_err(|e| e.to_string())?;

        let id = self.sched.add(job).await.map_err(|e| e.to_string())?;
        info!("Cron job added: {id} ({cron_expr})");
        Ok(id)
    }

    /// Starts scheduler background processing.
    pub async fn start(&self) -> Result<(), String> {
        self.sched.start().await.map_err(|e| e.to_string())?;
        info!("CronScheduler started");
        Ok(())
    }

    /// Gracefully shuts down the scheduler.
    pub async fn shutdown(&mut self) -> Result<(), String> {
        self.sched.shutdown().await.map_err(|e| e.to_string())?;
        info!("CronScheduler shutdown");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio::time::{Duration, timeout};

    use super::*;

    #[tokio::test]
    async fn add_job_rejects_invalid_cron_expression() {
        let sched = CronScheduler::new().await.expect("scheduler");
        let (tx, _rx) = mpsc::channel(2);
        let result = sched
            .add_job(
                "invalid cron",
                ChannelId::from("cli:local"),
                SessionId::from("s1"),
                "hello".to_string(),
                tx,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn add_job_start_and_shutdown_work() {
        let mut sched = CronScheduler::new().await.expect("scheduler");
        let (tx, mut rx) = mpsc::channel(4);
        let job_id = sched
            .add_job(
                "1/1 * * * * * *",
                ChannelId::from("cli:local"),
                SessionId::from("s2"),
                "tick".to_string(),
                tx,
            )
            .await
            .expect("job should be added");
        assert_ne!(job_id, uuid::Uuid::nil());

        sched.start().await.expect("scheduler starts");
        let event = timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("timeout should not elapse")
            .expect("event should be produced");
        assert_eq!(event.user_message, "tick");
        assert_eq!(event.channel_id.as_str(), "cli:local");

        sched.shutdown().await.expect("scheduler shutdown");
    }
}
