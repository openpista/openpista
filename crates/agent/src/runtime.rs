//! Runtime orchestration loop for conversation, tools, and memory.

use std::sync::Arc;

use proto::{AgentMessage, ChannelId, LlmError, Role, SessionId, WorkerReport};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    llm::{ChatMessage, ChatRequest, ChatResponse, LlmProvider},
    memory::SqliteMemory,
    tool_registry::ToolRegistry,
};

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are openpista, an OS Gateway AI Agent.
You can interact with the operating system through available tools.
Be helpful, concise, and safe. Always confirm before running potentially destructive commands."#;

/// The main agent runtime: manages the ReAct loop
pub struct AgentRuntime {
    llm: Arc<dyn LlmProvider>,
    tools: Arc<ToolRegistry>,
    memory: Arc<SqliteMemory>,
    model: String,
    max_tool_rounds: usize,
    worker_report_quic_addr: Option<String>,
}

impl AgentRuntime {
    /// Creates a new agent runtime with LLM provider, tools, and memory.
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<SqliteMemory>,
        model: impl Into<String>,
        max_tool_rounds: usize,
    ) -> Self {
        Self {
            llm,
            tools,
            memory,
            model: model.into(),
            max_tool_rounds,
            worker_report_quic_addr: None,
        }
    }

    /// Sets the orchestrator QUIC address injected into `container.run` calls.
    pub fn with_worker_report_quic_addr(mut self, addr: Option<String>) -> Self {
        self.worker_report_quic_addr = addr.and_then(|value| {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
        self
    }

    /// Process a user message and return the agent's final text response
    pub async fn process(
        &self,
        channel_id: &ChannelId,
        session_id: &SessionId,
        user_message: &str,
        skills_context: Option<&str>,
    ) -> Result<String, proto::Error> {
        // Ensure session exists
        self.memory
            .ensure_session(session_id, channel_id.as_str())
            .await
            .map_err(proto::Error::Database)?;

        // Save user message
        let user_msg = AgentMessage::new(session_id.clone(), Role::User, user_message);
        self.memory
            .save_message(&user_msg)
            .await
            .map_err(proto::Error::Database)?;

        // Build system prompt
        let system_prompt = build_system_prompt(skills_context);

        // Load conversation history
        let history = self
            .memory
            .load_session(session_id)
            .await
            .map_err(proto::Error::Database)?;

        // Convert history to chat messages
        let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(&system_prompt)];
        for msg in &history {
            match msg.role {
                Role::User => messages.push(ChatMessage::user(&msg.content)),
                Role::Assistant => {
                    let mut assistant = ChatMessage::assistant(&msg.content);
                    assistant.tool_calls = msg.tool_calls.clone();
                    messages.push(assistant);
                }
                Role::Tool => {
                    messages.push(ChatMessage::tool_result(
                        msg.tool_call_id.as_deref().unwrap_or(""),
                        msg.tool_name.as_deref().unwrap_or(""),
                        &msg.content,
                    ));
                }
                Role::System => {} // skip stored system messages
            }
        }

        // ReAct loop
        let tool_defs = self.tools.definitions();
        let mut round = 0;

        loop {
            if round >= self.max_tool_rounds {
                warn!(
                    "Max tool rounds ({}) reached for session {session_id}",
                    self.max_tool_rounds
                );
                return Err(proto::Error::Llm(LlmError::MaxToolRoundsExceeded));
            }

            let req = ChatRequest {
                messages: messages.clone(),
                tools: tool_defs.clone(),
                model: self.model.clone(),
            };

            debug!("LLM call (round {round}) for session {session_id}");
            let response = self.llm.chat(req).await.map_err(proto::Error::Llm)?;

            match response {
                ChatResponse::Text(text) => {
                    info!("Agent final response for session {session_id}: {text:.50}...");

                    // Save assistant response
                    let assistant_msg =
                        AgentMessage::new(session_id.clone(), Role::Assistant, &text);
                    self.memory
                        .save_message(&assistant_msg)
                        .await
                        .map_err(proto::Error::Database)?;

                    self.memory
                        .touch_session(session_id)
                        .await
                        .map_err(proto::Error::Database)?;

                    return Ok(text);
                }

                ChatResponse::ToolCalls(tool_calls) => {
                    debug!(
                        "Tool calls requested: {:?}",
                        tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>()
                    );

                    // Persist assistant tool-call message so replayed history remains valid.
                    let assistant_tool_calls_msg =
                        AgentMessage::assistant_tool_calls(session_id.clone(), tool_calls.clone());
                    self.memory
                        .save_message(&assistant_tool_calls_msg)
                        .await
                        .map_err(proto::Error::Database)?;

                    // Add assistant message with tool calls to history
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: String::new(),
                        tool_call_id: None,
                        tool_name: None,
                        tool_calls: Some(tool_calls.clone()),
                    };
                    messages.push(assistant_msg);

                    // Execute each tool call
                    for tc in &tool_calls {
                        let tool_args = prepare_tool_args(
                            &self.worker_report_quic_addr,
                            channel_id,
                            session_id,
                            &tc.name,
                            tc.arguments.clone(),
                        );

                        let result = self.tools.execute(&tc.id, &tc.name, tool_args).await;

                        // Save tool result message to memory
                        let tool_msg = AgentMessage::tool_result(
                            session_id.clone(),
                            &tc.id,
                            &tc.name,
                            &result.output,
                        );
                        self.memory
                            .save_message(&tool_msg)
                            .await
                            .map_err(proto::Error::Database)?;

                        // Add to in-memory conversation
                        messages.push(ChatMessage::tool_result(&tc.id, &tc.name, &result.output));
                    }

                    round += 1;
                }
            }
        }
    }

    /// Process a user message with real-time progress events.
    ///
    /// Identical to [`process()`](Self::process) but emits [`proto::ProgressEvent`]s
    /// on the provided channel so a TUI or other consumer can display
    /// live tool-call status while the ReAct loop runs.
    pub async fn process_with_progress(
        &self,
        channel_id: &ChannelId,
        session_id: &SessionId,
        user_message: &str,
        skills_context: Option<&str>,
        progress_tx: tokio::sync::mpsc::Sender<proto::ProgressEvent>,
    ) -> Result<String, proto::Error> {
        // Ensure session exists
        self.memory
            .ensure_session(session_id, channel_id.as_str())
            .await
            .map_err(proto::Error::Database)?;

        // Save user message
        let user_msg = AgentMessage::new(session_id.clone(), Role::User, user_message);
        self.memory
            .save_message(&user_msg)
            .await
            .map_err(proto::Error::Database)?;

        // Build system prompt
        let system_prompt = build_system_prompt(skills_context);

        // Load conversation history
        let history = self
            .memory
            .load_session(session_id)
            .await
            .map_err(proto::Error::Database)?;

        // Convert history to chat messages
        let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(&system_prompt)];
        for msg in &history {
            match msg.role {
                Role::User => messages.push(ChatMessage::user(&msg.content)),
                Role::Assistant => {
                    let mut assistant = ChatMessage::assistant(&msg.content);
                    assistant.tool_calls = msg.tool_calls.clone();
                    messages.push(assistant);
                }
                Role::Tool => {
                    messages.push(ChatMessage::tool_result(
                        msg.tool_call_id.as_deref().unwrap_or(""),
                        msg.tool_name.as_deref().unwrap_or(""),
                        &msg.content,
                    ));
                }
                Role::System => {} // skip stored system messages
            }
        }

        // ReAct loop with progress events
        let tool_defs = self.tools.definitions();
        let mut round = 0;

        loop {
            if round >= self.max_tool_rounds {
                warn!(
                    "Max tool rounds ({}) reached for session {session_id}",
                    self.max_tool_rounds
                );
                return Err(proto::Error::Llm(LlmError::MaxToolRoundsExceeded));
            }

            let req = ChatRequest {
                messages: messages.clone(),
                tools: tool_defs.clone(),
                model: self.model.clone(),
            };

            // Progress: LLM thinking
            let _ = progress_tx.try_send(proto::ProgressEvent::LlmThinking { round });

            debug!("LLM call (round {round}) for session {session_id}");
            let response = self.llm.chat(req).await.map_err(proto::Error::Llm)?;

            match response {
                ChatResponse::Text(text) => {
                    info!("Agent final response for session {session_id}: {text:.50}...");

                    // Save assistant response
                    let assistant_msg =
                        AgentMessage::new(session_id.clone(), Role::Assistant, &text);
                    self.memory
                        .save_message(&assistant_msg)
                        .await
                        .map_err(proto::Error::Database)?;

                    self.memory
                        .touch_session(session_id)
                        .await
                        .map_err(proto::Error::Database)?;

                    return Ok(text);
                }

                ChatResponse::ToolCalls(tool_calls) => {
                    debug!(
                        "Tool calls requested: {:?}",
                        tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>()
                    );

                    // Persist assistant tool-call message
                    let assistant_tool_calls_msg =
                        AgentMessage::assistant_tool_calls(session_id.clone(), tool_calls.clone());
                    self.memory
                        .save_message(&assistant_tool_calls_msg)
                        .await
                        .map_err(proto::Error::Database)?;

                    // Add assistant message with tool calls to history
                    let assistant_msg = ChatMessage {
                        role: Role::Assistant,
                        content: String::new(),
                        tool_call_id: None,
                        tool_name: None,
                        tool_calls: Some(tool_calls.clone()),
                    };
                    messages.push(assistant_msg);

                    // Execute each tool call with progress events
                    for tc in &tool_calls {
                        // Progress: tool call started
                        let _ = progress_tx.try_send(proto::ProgressEvent::ToolCallStarted {
                            call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            args: tc.arguments.clone(),
                        });

                        let tool_args = prepare_tool_args(
                            &self.worker_report_quic_addr,
                            channel_id,
                            session_id,
                            &tc.name,
                            tc.arguments.clone(),
                        );

                        let result = self.tools.execute(&tc.id, &tc.name, tool_args).await;

                        // Progress: tool call finished
                        let _ = progress_tx.try_send(proto::ProgressEvent::ToolCallFinished {
                            call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            output: result.output.clone(),
                            is_error: result.is_error,
                        });

                        // Save tool result message to memory
                        let tool_msg = AgentMessage::tool_result(
                            session_id.clone(),
                            &tc.id,
                            &tc.name,
                            &result.output,
                        );
                        self.memory
                            .save_message(&tool_msg)
                            .await
                            .map_err(proto::Error::Database)?;

                        // Add to in-memory conversation
                        messages.push(ChatMessage::tool_result(&tc.id, &tc.name, &result.output));
                    }

                    round += 1;
                }
            }
        }
    }

    /// Persists a worker report as a tool-result message in the target session.
    pub async fn record_worker_report(
        &self,
        channel_id: &ChannelId,
        session_id: &SessionId,
        report: &WorkerReport,
    ) -> Result<(), proto::Error> {
        self.memory
            .ensure_session(session_id, channel_id.as_str())
            .await
            .map_err(proto::Error::Database)?;

        let tool_name = format!("container.worker.{}", report.worker_id);
        let msg = AgentMessage::tool_result(
            session_id.clone(),
            report.call_id.clone(),
            tool_name,
            report.output.clone(),
        );

        self.memory
            .save_message(&msg)
            .await
            .map_err(proto::Error::Database)?;
        self.memory
            .touch_session(session_id)
            .await
            .map_err(proto::Error::Database)?;
        Ok(())
    }
}

/// Builds the effective system prompt with optional skills context section.
fn build_system_prompt(skills_context: Option<&str>) -> String {
    let mut prompt = DEFAULT_SYSTEM_PROMPT.to_string();
    if let Some(skills) = skills_context
        && !skills.is_empty()
    {
        prompt.push_str("\n\n## Available Skills\n\n");
        prompt.push_str(skills);
    }
    prompt
}

fn prepare_tool_args(
    worker_report_quic_addr: &Option<String>,
    channel_id: &ChannelId,
    session_id: &SessionId,
    tool_name: &str,
    args: Value,
) -> Value {
    if tool_name != "container.run" {
        return args;
    }

    let mut object = match args {
        Value::Object(map) => map,
        other => return other,
    };

    object.insert("allow_subprocess_fallback".to_string(), Value::Bool(false));
    object.insert(
        "orchestrator_quic_insecure_skip_verify".to_string(),
        Value::Bool(false),
    );

    let Some(quic_addr) = worker_report_quic_addr.as_deref() else {
        return Value::Object(object);
    };

    object.insert("report_via_quic".to_string(), Value::Bool(true));
    object.insert(
        "orchestrator_quic_addr".to_string(),
        Value::String(quic_addr.to_string()),
    );
    object.insert(
        "orchestrator_channel_id".to_string(),
        Value::String(channel_id.as_str().to_string()),
    );
    object.insert(
        "orchestrator_session_id".to_string(),
        Value::String(session_id.as_str().to_string()),
    );

    Value::Object(object)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use async_trait::async_trait;
    use proto::{LlmError, ToolCall, ToolResult};

    use super::*;

    struct MockLlm {
        queue: Mutex<VecDeque<ChatResponse>>,
    }

    impl MockLlm {
        fn new(responses: Vec<ChatResponse>) -> Self {
            Self {
                queue: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockLlm {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, LlmError> {
            self.queue
                .lock()
                .expect("lock queue")
                .pop_front()
                .ok_or_else(|| LlmError::InvalidResponse("No mock response left".to_string()))
        }
    }

    struct EchoTool;

    struct SlowLlm {
        delay: std::time::Duration,
    }

    #[async_trait]
    impl tools::Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echo test tool"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type":"object",
                "properties":{"value":{"type":"string"}},
                "required":["value"]
            })
        }

        async fn execute(&self, call_id: &str, args: serde_json::Value) -> ToolResult {
            let value = args["value"].as_str().unwrap_or_default();
            ToolResult::success(call_id, self.name(), format!("echo:{value}"))
        }
    }

    #[async_trait]
    impl LlmProvider for SlowLlm {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, LlmError> {
            tokio::time::sleep(self.delay).await;
            Ok(ChatResponse::Text("late".to_string()))
        }
    }

    async fn open_temp_memory() -> Arc<SqliteMemory> {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let db_path = tempdir.path().join("memory.db");
        let db_path_str = db_path.to_string_lossy().to_string();
        let memory = SqliteMemory::open(&db_path_str).await.expect("memory open");
        // Keep tempdir alive for test process lifetime.
        std::mem::forget(tempdir);
        Arc::new(memory)
    }

    fn build_registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);
        Arc::new(registry)
    }

    #[tokio::test]
    async fn process_returns_text_and_persists_messages() {
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "assistant reply".to_string(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory.clone(), "mock-model", 4);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-1");

        let text = runtime
            .process(&channel, &session, "hello", None)
            .await
            .expect("process should succeed");
        assert_eq!(text, "assistant reply");

        let history = memory.load_session(&session).await.expect("history");
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, Role::User);
        assert_eq!(history[0].content, "hello");
        assert_eq!(history[1].role, Role::Assistant);
        assert_eq!(history[1].content, "assistant reply");
    }

    #[tokio::test]
    async fn process_executes_tool_calls_then_returns_text() {
        let tool_call = ToolCall {
            id: "call-1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value":"pong"}),
        };
        let llm = Arc::new(MockLlm::new(vec![
            ChatResponse::ToolCalls(vec![tool_call]),
            ChatResponse::Text("done".to_string()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory.clone(), "mock-model", 4);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-2");

        let text = runtime
            .process(&channel, &session, "run echo", Some("skill context"))
            .await
            .expect("process should succeed");
        assert_eq!(text, "done");

        let history = memory.load_session(&session).await.expect("history");
        assert_eq!(history.len(), 4);
        assert_eq!(history[0].role, Role::User);
        assert_eq!(history[1].role, Role::Assistant);
        assert_eq!(history[1].content, "");
        assert_eq!(history[1].tool_calls.as_ref().map(Vec::len), Some(1));
        assert_eq!(history[2].role, Role::Tool);
        assert_eq!(history[2].content, "echo:pong");
        assert_eq!(history[3].role, Role::Assistant);
        assert_eq!(history[3].content, "done");
    }

    #[tokio::test]
    async fn process_errors_when_max_tool_rounds_exceeded() {
        let tool_call = ToolCall {
            id: "call-2".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value":"x"}),
        };
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::ToolCalls(vec![tool_call])]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "mock-model", 1);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-3");

        let err = runtime
            .process(&channel, &session, "loop", None)
            .await
            .expect_err("should exceed rounds");
        match err {
            proto::Error::Llm(LlmError::MaxToolRoundsExceeded) => {}
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn process_propagates_llm_provider_error() {
        let llm = Arc::new(MockLlm::new(Vec::new()));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "mock-model", 2);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-llm-error");

        let err = runtime
            .process(&channel, &session, "hello", None)
            .await
            .expect_err("llm provider error should propagate");

        match err {
            proto::Error::Llm(LlmError::InvalidResponse(msg)) => {
                assert!(msg.contains("No mock response left"));
            }
            other => panic!("unexpected error: {other}"),
        }
    }

    #[tokio::test]
    async fn process_can_be_bounded_with_timeout_for_slow_provider() {
        let llm = Arc::new(SlowLlm {
            delay: std::time::Duration::from_millis(200),
        });
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "mock-model", 2);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-llm-timeout");

        let timed = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            runtime.process(&channel, &session, "hello", None),
        )
        .await;

        assert!(timed.is_err());
    }

    #[tokio::test]
    async fn process_with_progress_emits_thinking_and_returns_text() {
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::Text("done".to_string())]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "mock-model", 4);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-progress-1");
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);

        let text = runtime
            .process_with_progress(&channel, &session, "hello", None, tx)
            .await
            .expect("process_with_progress should succeed");
        assert_eq!(text, "done");

        let first = rx.recv().await.expect("thinking event");
        assert!(matches!(
            first,
            proto::ProgressEvent::LlmThinking { round: 0 }
        ));
    }

    #[tokio::test]
    async fn process_with_progress_emits_tool_start_and_finish_events() {
        let tool_call = ToolCall {
            id: "call-progress-1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value":"pong"}),
        };
        let llm = Arc::new(MockLlm::new(vec![
            ChatResponse::ToolCalls(vec![tool_call]),
            ChatResponse::Text("final".to_string()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "mock-model", 4);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-progress-2");
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);

        let text = runtime
            .process_with_progress(&channel, &session, "run tool", Some("skills"), tx)
            .await
            .expect("process_with_progress should succeed");
        assert_eq!(text, "final");

        let mut started = false;
        let mut finished = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                proto::ProgressEvent::ToolCallStarted {
                    call_id,
                    tool_name,
                    args,
                } => {
                    assert_eq!(call_id, "call-progress-1");
                    assert_eq!(tool_name, "echo");
                    assert_eq!(args["value"], "pong");
                    started = true;
                }
                proto::ProgressEvent::ToolCallFinished {
                    call_id,
                    tool_name,
                    output,
                    is_error,
                } => {
                    assert_eq!(call_id, "call-progress-1");
                    assert_eq!(tool_name, "echo");
                    assert_eq!(output, "echo:pong");
                    assert!(!is_error);
                    finished = true;
                }
                proto::ProgressEvent::LlmThinking { .. } => {}
            }
        }
        assert!(started);
        assert!(finished);
    }

    #[test]
    fn prepare_tool_args_enforces_worker_report_metadata_for_container_tool() {
        let args = prepare_tool_args(
            &Some("127.0.0.1:4433".to_string()),
            &ChannelId::from("cli:local"),
            &SessionId::from("session-x"),
            "container.run",
            serde_json::json!({
                "image":"alpine:3",
                "command":"echo hi",
                "report_via_quic": false,
                "orchestrator_quic_addr": "8.8.8.8:1111",
                "orchestrator_channel_id": "attacker:channel",
                "orchestrator_session_id": "attacker-session"
            }),
        );

        assert_eq!(args["report_via_quic"], true);
        assert_eq!(args["allow_subprocess_fallback"], false);
        assert_eq!(args["orchestrator_quic_insecure_skip_verify"], false);
        assert_eq!(args["orchestrator_quic_addr"], "127.0.0.1:4433");
        assert_eq!(args["orchestrator_channel_id"], "cli:local");
        assert_eq!(args["orchestrator_session_id"], "session-x");
    }

    #[test]
    fn prepare_tool_args_enforces_local_safety_flags_without_quic_addr() {
        let args = prepare_tool_args(
            &None,
            &ChannelId::from("cli:local"),
            &SessionId::from("session-x"),
            "container.run",
            serde_json::json!({
                "image":"alpine:3",
                "command":"echo hi",
                "allow_subprocess_fallback": true,
                "orchestrator_quic_insecure_skip_verify": true
            }),
        );

        assert_eq!(args["allow_subprocess_fallback"], false);
        assert_eq!(args["orchestrator_quic_insecure_skip_verify"], false);
        assert!(args["report_via_quic"].is_null());
    }

    #[tokio::test]
    async fn record_worker_report_persists_tool_message() {
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::Text("done".to_string())]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory.clone(), "mock-model", 4);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-worker-report");
        let report = proto::WorkerReport::new(
            "call-42",
            "worker-1",
            "alpine:3.20",
            "echo hi",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "hi
"
                .into(),
                stderr: "".into(),
                output: "stdout:
hi

exit_code: 0"
                    .into(),
            },
        );

        runtime
            .record_worker_report(&channel, &session, &report)
            .await
            .expect("worker report should be recorded");

        let history = memory.load_session(&session).await.expect("history");
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].role, Role::Tool);
        assert_eq!(history[0].tool_call_id.as_deref(), Some("call-42"));
        assert!(
            history[0]
                .tool_name
                .as_deref()
                .unwrap_or_default()
                .contains("worker-1")
        );
        assert_eq!(history[0].content, report.output);
    }

    #[test]
    fn build_system_prompt_includes_skills_when_non_empty() {
        let prompt = build_system_prompt(Some("### Skill: demo"));
        assert!(prompt.contains("Available Skills"));
        assert!(prompt.contains("### Skill: demo"));
    }

    #[test]
    fn build_system_prompt_skips_empty_skills() {
        let base = build_system_prompt(None);
        assert!(!base.contains("Available Skills"));

        let empty = build_system_prompt(Some(""));
        assert!(!empty.contains("Available Skills"));
    }
}
