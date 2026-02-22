use proto::{AgentMessage, DatabaseError, Role, SessionId};
use sqlx::{Row, sqlite::SqlitePool};
use std::str::FromStr;
use tracing::{debug, info};

/// SQLite-backed conversation memory
pub struct SqliteMemory {
    pool: SqlitePool,
}

impl SqliteMemory {
    /// Open (or create) the SQLite database and run migrations
    pub async fn open(db_url: &str) -> Result<Self, DatabaseError> {
        // Expand ~ in path
        let url = if db_url.starts_with("~") {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
            db_url.replacen("~", &home, 1)
        } else {
            db_url.to_string()
        };

        // Ensure parent directory exists
        if let Some(parent) = std::path::Path::new(&url).parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;
        }

        let pool = SqlitePool::connect(&format!("sqlite:{url}?mode=rwc"))
            .await
            .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        let migrations_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let migrator = sqlx::migrate::Migrator::new(migrations_dir.as_path())
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;

        migrator
            .run(&pool)
            .await
            .map_err(|e| DatabaseError::Migration(e.to_string()))?;

        info!("SQLite memory opened: {url}");
        Ok(Self { pool })
    }

    /// Save a message to the database
    pub async fn save_message(&self, msg: &AgentMessage) -> Result<(), DatabaseError> {
        let tool_calls_json = msg
            .tool_calls
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        sqlx::query(
            "INSERT OR REPLACE INTO messages (id, session_id, role, content, tool_call_id, tool_name, tool_calls_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(&msg.id)
        .bind(msg.session_id.as_str())
        .bind(msg.role.to_string())
        .bind(&msg.content)
        .bind(&msg.tool_call_id)
        .bind(&msg.tool_name)
        .bind(tool_calls_json)
        .bind(msg.created_at.to_rfc3339())
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        debug!("Saved message: {} (role: {})", msg.id, msg.role);
        Ok(())
    }

    /// Ensure a session exists (create if not)
    pub async fn ensure_session(
        &self,
        session_id: &SessionId,
        channel_id: &str,
    ) -> Result<(), DatabaseError> {
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT OR IGNORE INTO sessions (id, channel_id, created_at, updated_at) VALUES (?, ?, ?, ?)"
        )
        .bind(session_id.as_str())
        .bind(channel_id)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;
        Ok(())
    }

    /// Load all messages for a session (ordered by created_at)
    pub async fn load_session(
        &self,
        session_id: &SessionId,
    ) -> Result<Vec<AgentMessage>, DatabaseError> {
        let rows = sqlx::query(
            "SELECT id, session_id, role, content, tool_call_id, tool_name, tool_calls_json, created_at FROM messages WHERE session_id = ? ORDER BY created_at ASC"
        )
        .bind(session_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        let messages = rows
            .into_iter()
            .map(|row| {
                let role_str: String = row.get("role");
                let role = Role::from_str(&role_str).unwrap_or(Role::User);
                let created_at_str: String = row.get("created_at");
                let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let tool_calls_json: Option<String> = row.get("tool_calls_json");
                let tool_calls = tool_calls_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Vec<proto::ToolCall>>(raw).ok());

                AgentMessage {
                    id: row.get("id"),
                    session_id: SessionId::from(row.get::<String, _>("session_id")),
                    role,
                    content: row.get("content"),
                    tool_call_id: row.get("tool_call_id"),
                    tool_name: row.get("tool_name"),
                    tool_calls,
                    created_at,
                }
            })
            .collect();

        Ok(messages)
    }

    /// List all sessions
    pub async fn list_sessions(&self) -> Result<Vec<(SessionId, String)>, DatabaseError> {
        let rows = sqlx::query("SELECT id, channel_id FROM sessions ORDER BY updated_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    SessionId::from(row.get::<String, _>("id")),
                    row.get::<String, _>("channel_id"),
                )
            })
            .collect())
    }

    pub async fn list_sessions_with_preview(
        &self,
    ) -> Result<Vec<(SessionId, String, chrono::DateTime<chrono::Utc>, String)>, DatabaseError>
    {
        let rows = sqlx::query(
            r#"SELECT s.id, s.channel_id, s.updated_at,
                      COALESCE(
                        (SELECT content FROM messages
                         WHERE session_id = s.id AND role = 'user'
                         ORDER BY created_at ASC LIMIT 1),
                        ''
                      ) AS preview
               FROM sessions s
               ORDER BY s.updated_at DESC"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;

        Ok(rows
            .into_iter()
            .map(|row| {
                let updated_str: String = row.get("updated_at");
                let updated = chrono::DateTime::parse_from_rfc3339(&updated_str)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| chrono::Utc::now());
                let preview: String = row.get("preview");
                (
                    SessionId::from(row.get::<String, _>("id")),
                    row.get::<String, _>("channel_id"),
                    updated,
                    preview,
                )
            })
            .collect())
    }

    pub async fn touch_session(&self, session_id: &SessionId) -> Result<(), DatabaseError> {
        sqlx::query("UPDATE sessions SET updated_at = ? WHERE id = ?")
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(session_id.as_str())
            .execute(&self.pool)
            .await
            .map_err(|e| DatabaseError::Sqlx(e.to_string()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    async fn open_temp_memory() -> (SqliteMemory, tempfile::TempDir, PathBuf) {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let db_path = tempdir.path().join("memory.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let memory = SqliteMemory::open(&db_path_str)
            .await
            .expect("memory should open");
        (memory, tempdir, db_path)
    }

    #[tokio::test]
    async fn open_creates_database_and_parent_dir() {
        let (memory, _tmp, db_path) = open_temp_memory().await;
        assert!(db_path.exists());
        drop(memory);
    }

    #[tokio::test]
    async fn ensure_session_is_idempotent_and_listed() {
        let (memory, _tmp, _path) = open_temp_memory().await;
        let session_id = SessionId::from("session-a");

        memory
            .ensure_session(&session_id, "cli:local")
            .await
            .expect("ensure session first call");
        memory
            .ensure_session(&session_id, "cli:local")
            .await
            .expect("ensure session second call");

        let sessions = memory.list_sessions().await.expect("list sessions");
        assert!(
            sessions
                .iter()
                .any(|(id, channel)| id.as_str() == "session-a" && channel == "cli:local")
        );
    }

    #[tokio::test]
    async fn save_and_load_messages_round_trip() {
        let (memory, _tmp, _path) = open_temp_memory().await;
        let session_id = SessionId::from("session-b");
        memory
            .ensure_session(&session_id, "cli:local")
            .await
            .expect("ensure session");

        let user = AgentMessage::new(session_id.clone(), Role::User, "hello");
        let assistant = AgentMessage::assistant_tool_calls(
            session_id.clone(),
            vec![proto::ToolCall {
                id: "call-assistant".to_string(),
                name: "system.run".to_string(),
                arguments: serde_json::json!({"command":"echo hi"}),
            }],
        );
        let tool = AgentMessage::tool_result(session_id.clone(), "call-1", "system.run", "ok");
        memory.save_message(&user).await.expect("save user");
        memory
            .save_message(&assistant)
            .await
            .expect("save assistant tool calls");
        memory.save_message(&tool).await.expect("save tool");

        let loaded = memory
            .load_session(&session_id)
            .await
            .expect("load session");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].content, "hello");
        assert_eq!(loaded[0].role, Role::User);
        assert_eq!(loaded[1].role, Role::Assistant);
        assert_eq!(loaded[1].content, "");
        assert_eq!(loaded[1].tool_calls.as_ref().map(Vec::len), Some(1));
        assert_eq!(loaded[2].role, Role::Tool);
        assert_eq!(loaded[2].tool_call_id.as_deref(), Some("call-1"));
        assert_eq!(loaded[2].tool_name.as_deref(), Some("system.run"));
    }

    #[tokio::test]
    async fn touch_session_updates_without_error() {
        let (memory, _tmp, _path) = open_temp_memory().await;
        let session_id = SessionId::from("session-c");
        memory
            .ensure_session(&session_id, "cli:local")
            .await
            .expect("ensure session");
        memory
            .touch_session(&session_id)
            .await
            .expect("touch session should succeed");
    }
}
