//! Runtime orchestration loop for conversation, tools, and memory.

use std::sync::Arc;

use proto::{AgentMessage, ChannelId, LlmError, Role, SessionId};
use serde_json::Value;
use tracing::{debug, info, warn};

use crate::{
    llm::{ChatMessage, ChatRequest, ChatResponse, LlmProvider, TokenUsage},
    memory::SqliteMemory,
    tool_registry::ToolRegistry,
};

const DEFAULT_SYSTEM_PROMPT: &str = r#"You are openpista, an OS Gateway AI Agent.
You can interact with the operating system through available tools.
Be helpful, concise, and safe. Always confirm before running potentially destructive commands."#;
const MAX_CONTEXT_MESSAGES: usize = 40;
const MAX_TOOL_RESULT_CHARS: usize = 16_000;

/// The main agent runtime: manages the ReAct loop
pub struct AgentRuntime {
    llm: std::sync::RwLock<Arc<dyn LlmProvider>>,
    providers: std::sync::RwLock<std::collections::HashMap<String, Arc<dyn LlmProvider>>>,
    active_provider: std::sync::RwLock<String>,
    tools: Arc<ToolRegistry>,
    memory: Arc<SqliteMemory>,
    model: std::sync::RwLock<String>,
    max_tool_rounds: usize,
}

impl AgentRuntime {
    /// Creates a new agent runtime with LLM provider, tools, and memory.
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        memory: Arc<SqliteMemory>,
        provider_name: &str,
        model: impl Into<String>,
        max_tool_rounds: usize,
    ) -> Self {
        let mut providers = std::collections::HashMap::new();
        providers.insert(provider_name.to_string(), Arc::clone(&llm));
        Self {
            llm: std::sync::RwLock::new(llm),
            providers: std::sync::RwLock::new(providers),
            active_provider: std::sync::RwLock::new(provider_name.to_string()),
            tools,
            memory,
            model: std::sync::RwLock::new(model.into()),
            max_tool_rounds,
        }
    }

    pub fn memory(&self) -> &Arc<SqliteMemory> {
        &self.memory
    }

    pub fn set_model(&self, model: String) {
        *self.model.write().expect("model lock") = model;
    }

    /// Replaces the active LLM provider (e.g. after switching from OpenAI to Anthropic).
    pub fn set_llm(&self, llm: Arc<dyn LlmProvider>) {
        *self.llm.write().expect("llm lock") = llm;
    }

    /// Registers an additional LLM provider by name.
    pub fn register_provider(&self, name: &str, llm: Arc<dyn LlmProvider>) {
        self.providers
            .write()
            .expect("providers lock")
            .insert(name.to_string(), llm);
    }

    /// Switches the active LLM provider to a previously registered one.
    pub fn switch_provider(&self, name: &str) -> Result<(), String> {
        let providers = self.providers.read().expect("providers lock");
        let provider = providers
            .get(name)
            .ok_or_else(|| format!("unknown provider: {name}"))?;
        *self.llm.write().expect("llm lock") = Arc::clone(provider);
        *self.active_provider.write().expect("active_provider lock") = name.to_string();
        Ok(())
    }

    /// Returns the name of the currently active provider.
    pub fn active_provider_name(&self) -> String {
        self.active_provider
            .read()
            .expect("active_provider lock")
            .clone()
    }

    /// Returns the names of all registered providers.
    pub fn registered_providers(&self) -> Vec<String> {
        let providers = self.providers.read().expect("providers lock");
        let mut names: Vec<String> = providers.keys().cloned().collect();
        names.sort();
        names
    }

    /// Process a user message and return the agent's final text response
    pub async fn process(
        &self,
        channel_id: &ChannelId,
        session_id: &SessionId,
        user_message: &str,
        skills_context: Option<&str>,
    ) -> Result<(String, TokenUsage), proto::Error> {
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

        let history = trim_session_history(history);

        let mut messages = history_to_chat_messages(&system_prompt, &history);

        // ReAct loop
        let tool_defs = self.tools.definitions();
        let mut round = 0;
        let mut total_usage = TokenUsage::default();

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
                model: self.model.read().expect("model lock").clone(),
            };
            debug!("LLM call (round {round}) for session {session_id}");
            let llm = Arc::clone(&*self.llm.read().expect("llm lock"));
            let t0 = std::time::Instant::now();
            let response = llm.chat(req).await.map_err(proto::Error::Llm)?;
            debug!(elapsed_ms = %t0.elapsed().as_millis(), round = %round, "LLM response received");
            match response {
                ChatResponse::Text(text, usage) => {
                    info!("Agent final response for session {session_id}: {text:.50}...");
                    total_usage.add(&usage);
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

                    return Ok((text, total_usage));
                }

                ChatResponse::ToolCalls(tool_calls, usage) => {
                    debug!(
                        "Tool calls requested: {:?}",
                        tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>()
                    );
                    total_usage.add(&usage);
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
                    for tc in &tool_calls {
                        let tool_args = prepare_tool_args(&tc.name, tc.arguments.clone());
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
                        messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            truncate_tool_result(&result.output, MAX_TOOL_RESULT_CHARS),
                        ));
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

        let history = trim_session_history(history);

        let mut messages = history_to_chat_messages(&system_prompt, &history);

        // ReAct loop with progress events
        let tool_defs = self.tools.definitions();
        let mut round = 0;
        let mut total_usage = TokenUsage::default();

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
                model: self.model.read().expect("model lock").clone(),
            };

            // Progress: LLM thinking
            let _ = progress_tx.try_send(proto::ProgressEvent::LlmThinking { round });

            debug!("LLM call (round {round}) for session {session_id}");
            let llm = Arc::clone(&*self.llm.read().expect("llm lock"));
            let t0 = std::time::Instant::now();
            let response = llm.chat(req).await.map_err(proto::Error::Llm)?;
            debug!(elapsed_ms = %t0.elapsed().as_millis(), round = %round, "LLM response received");

            match response {
                ChatResponse::Text(text, usage) => {
                    info!("Agent final response for session {session_id}: {text:.50}...");
                    total_usage.add(&usage);
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
                    info!(
                        prompt_tokens = total_usage.prompt_tokens,
                        completion_tokens = total_usage.completion_tokens,
                        "Accumulated token usage in process_with_progress"
                    );
                    return Ok(text);
                }

                ChatResponse::ToolCalls(tool_calls, usage) => {
                    debug!(
                        "Tool calls requested: {:?}",
                        tool_calls.iter().map(|tc| &tc.name).collect::<Vec<_>>()
                    );
                    total_usage.add(&usage);
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
                    for tc in &tool_calls {
                        // Progress: tool call started
                        let _ = progress_tx.try_send(proto::ProgressEvent::ToolCallStarted {
                            call_id: tc.id.clone(),
                            tool_name: tc.name.clone(),
                            args: tc.arguments.clone(),
                        });

                        let tool_args = prepare_tool_args(&tc.name, tc.arguments.clone());

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
                        messages.push(ChatMessage::tool_result(
                            &tc.id,
                            &tc.name,
                            truncate_tool_result(&result.output, MAX_TOOL_RESULT_CHARS),
                        ));
                    }
                    round += 1;
                }
            }
        }
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

/// Trims loaded session history to stay within context limits while preserving
/// message-sequence integrity around user boundaries.
fn trim_session_history(history: Vec<AgentMessage>) -> Vec<AgentMessage> {
    if history.len() <= MAX_CONTEXT_MESSAGES {
        return history;
    }

    let start = history.len() - MAX_CONTEXT_MESSAGES;
    // Advance to next User boundary to preserve tool-call integrity.
    let offset = history[start..]
        .iter()
        .position(|m| m.role == Role::User)
        .unwrap_or(0);
    history[start + offset..].to_vec()
}

/// Converts persisted session history into model input messages, including
/// tool-output truncation safeguards.
fn history_to_chat_messages(system_prompt: &str, history: &[AgentMessage]) -> Vec<ChatMessage> {
    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(system_prompt)];
    for msg in history {
        match msg.role {
            Role::User => messages.push(ChatMessage::user(&msg.content)),
            Role::Assistant => {
                let mut assistant = ChatMessage::assistant(&msg.content);
                assistant.tool_calls = msg.tool_calls.clone();
                messages.push(assistant);
            }
            Role::Tool => {
                let content = truncate_tool_result(&msg.content, MAX_TOOL_RESULT_CHARS);
                messages.push(ChatMessage::tool_result(
                    msg.tool_call_id.as_deref().unwrap_or(""),
                    msg.tool_name.as_deref().unwrap_or(""),
                    &content,
                ));
            }
            Role::System => {} // skip stored system messages
        }
    }
    messages
}

fn prepare_tool_args(tool_name: &str, args: Value) -> Value {
    if tool_name != "container.run" {
        return args;
    }

    let mut object = match args {
        Value::Object(map) => map,
        other => return other,
    };

    object.insert("allow_subprocess_fallback".to_string(), Value::Bool(false));

    Value::Object(object)
}

/// Truncates a tool result to at most `max_chars` characters.
/// If the result is longer, it appends a note with how many characters were cut.
fn truncate_tool_result(output: &str, max_chars: usize) -> String {
    let total_chars = output.chars().count();
    if total_chars <= max_chars {
        return output.to_string();
    }

    let kept = output.chars().take(max_chars).collect::<String>();
    let cut = total_chars - max_chars;
    format!("{kept}\n...[output truncated: {cut} chars omitted]")
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
            Ok(ChatResponse::Text(
                "late".to_string(),
                TokenUsage::default(),
            ))
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
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory.clone(),
            "mock-provider",
            "mock-model",
            4,
        );
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-1");

        let (text, _usage) = runtime
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
            ChatResponse::ToolCalls(vec![tool_call], TokenUsage::default()),
            ChatResponse::Text("done".to_string(), TokenUsage::default()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory.clone(),
            "mock-provider",
            "mock-model",
            4,
        );
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-2");

        let (text, _usage) = runtime
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
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::ToolCalls(
            vec![tool_call],
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            1,
        );
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
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            2,
        );
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
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            2,
        );
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
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "done".to_string(),
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            4,
        );
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
            ChatResponse::ToolCalls(vec![tool_call], TokenUsage::default()),
            ChatResponse::Text("final".to_string(), TokenUsage::default()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            4,
        );
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
    fn prepare_tool_args_enforces_safety_flags_for_container_tool() {
        let args = prepare_tool_args(
            "container.run",
            serde_json::json!({
                "image":"alpine:3",
                "command":"echo hi",
                "allow_subprocess_fallback": true
            }),
        );

        assert_eq!(args["allow_subprocess_fallback"], false);
    }

    #[test]
    fn prepare_tool_args_passes_through_non_container_tools() {
        let input = serde_json::json!({"value":"hello"});
        let args = prepare_tool_args("echo", input.clone());
        assert_eq!(args, input);
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

    #[tokio::test]
    async fn register_and_switch_provider() {
        let llm1 = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "from-first".to_string(),
            TokenUsage::default(),
        )]));
        let llm2 = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "from-second".to_string(),
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm1, build_registry(), memory, "first", "mock-model", 4);
        assert_eq!(runtime.active_provider_name(), "first");

        runtime.register_provider("second", llm2);
        runtime
            .switch_provider("second")
            .expect("switch should succeed");
        assert_eq!(runtime.active_provider_name(), "second");

        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-switch");
        let (text, _usage) = runtime
            .process(&channel, &session, "hello", None)
            .await
            .expect("process should succeed");
        assert_eq!(text, "from-second");
    }

    #[test]
    fn switch_unknown_provider_fails() {
        let llm = Arc::new(MockLlm::new(vec![]));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let memory = rt.block_on(open_temp_memory());
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "default", "m", 1);
        let err = runtime.switch_provider("nonexistent");
        assert!(err.is_err());
        assert!(err.unwrap_err().contains("unknown provider"));
    }

    #[test]
    fn registered_providers_returns_all_names() {
        let llm1 = Arc::new(MockLlm::new(vec![]));
        let llm2 = Arc::new(MockLlm::new(vec![]));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let memory = rt.block_on(open_temp_memory());
        let runtime = AgentRuntime::new(llm1, build_registry(), memory, "alpha", "m", 1);
        runtime.register_provider("beta", llm2);
        let names = runtime.registered_providers();
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    // ── truncate_tool_result ─────────────────────────────────────────

    #[test]
    fn truncate_tool_result_short_input_unchanged() {
        let result = truncate_tool_result("hello", 100);
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_tool_result_exact_boundary_unchanged() {
        let input = "a".repeat(50);
        let result = truncate_tool_result(&input, 50);
        assert_eq!(result, input);
    }

    #[test]
    fn truncate_tool_result_over_limit_truncates_with_note() {
        let input = "a".repeat(100);
        let result = truncate_tool_result(&input, 60);
        assert!(result.starts_with(&"a".repeat(60)));
        assert!(result.contains("output truncated"));
        assert!(result.contains("40 chars omitted"));
    }

    #[test]
    fn truncate_tool_result_multibyte_is_utf8_safe() {
        let input = "안녕🙂세계";
        let result = truncate_tool_result(input, 3);
        assert!(result.starts_with("안녕🙂"));
        assert!(result.contains("2 chars omitted"));
    }

    #[test]
    fn truncate_tool_result_empty_input() {
        let result = truncate_tool_result("", 100);
        assert_eq!(result, "");
    }

    #[test]
    fn truncate_tool_result_zero_limit() {
        let result = truncate_tool_result("hello", 0);
        assert!(result.contains("output truncated"));
        assert!(result.contains("5 chars omitted"));
    }

    // ── prepare_tool_args additional ─────────────────────────────────

    #[test]
    fn prepare_tool_args_non_object_passthrough() {
        let args = serde_json::json!("just a string");
        let result = prepare_tool_args("container.run", args.clone());
        assert_eq!(result, args);
    }

    #[test]
    fn prepare_tool_args_container_adds_flag() {
        let args = serde_json::json!({"image": "ubuntu"});
        let result = prepare_tool_args("container.run", args);
        assert_eq!(result["allow_subprocess_fallback"], false);
        assert_eq!(result["image"], "ubuntu");
    }

    // ── getter/setter coverage ─────────────────────────────────────

    #[tokio::test]
    async fn memory_getter_returns_shared_memory() {
        let llm = Arc::new(MockLlm::new(vec![]));
        let memory = open_temp_memory().await;
        let memory_clone = Arc::clone(&memory);
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "p", "m", 1);
        assert!(Arc::ptr_eq(runtime.memory(), &memory_clone));
    }

    #[tokio::test]
    async fn set_model_changes_active_model() {
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "ok".to_string(),
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm, build_registry(), memory, "p", "old-model", 4);
        runtime.set_model("new-model".to_string());
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-set-model");
        let (text, _) = runtime
            .process(&channel, &session, "hi", None)
            .await
            .expect("process");
        assert_eq!(text, "ok");
    }

    #[tokio::test]
    async fn set_llm_replaces_active_provider() {
        let llm1 = Arc::new(MockLlm::new(vec![]));
        let llm2 = Arc::new(MockLlm::new(vec![ChatResponse::Text(
            "from-new".to_string(),
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(llm1, build_registry(), memory, "p", "m", 4);
        runtime.set_llm(llm2);
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-set-llm");
        let (text, _) = runtime
            .process(&channel, &session, "hi", None)
            .await
            .expect("process");
        assert_eq!(text, "from-new");
    }

    // ── history role conversion coverage ──────────────────────────────

    #[tokio::test]
    async fn process_converts_prior_assistant_and_tool_history() {
        let tool_call = ToolCall {
            id: "tc-hist".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value": "test"}),
        };
        let llm = Arc::new(MockLlm::new(vec![
            ChatResponse::ToolCalls(vec![tool_call], TokenUsage::default()),
            ChatResponse::Text("first-done".to_string(), TokenUsage::default()),
            ChatResponse::Text("second-done".to_string(), TokenUsage::default()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory.clone(),
            "mock-provider",
            "mock-model",
            4,
        );
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-history-conv");

        let (text1, _) = runtime
            .process(&channel, &session, "first", None)
            .await
            .expect("first process");
        assert_eq!(text1, "first-done");

        let (text2, _) = runtime
            .process(&channel, &session, "second", None)
            .await
            .expect("second process");
        assert_eq!(text2, "second-done");

        let history = memory.load_session(&session).await.expect("history");
        assert_eq!(history.len(), 6);
    }

    #[tokio::test]
    async fn process_with_progress_converts_prior_assistant_and_tool_history() {
        let tool_call = ToolCall {
            id: "tc-prog-hist".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value": "test"}),
        };
        let llm = Arc::new(MockLlm::new(vec![
            ChatResponse::ToolCalls(vec![tool_call], TokenUsage::default()),
            ChatResponse::Text("first-done".to_string(), TokenUsage::default()),
            ChatResponse::Text("second-done".to_string(), TokenUsage::default()),
        ]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            4,
        );
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-prog-hist");

        let (tx1, _rx1) = tokio::sync::mpsc::channel(32);
        let text1 = runtime
            .process_with_progress(&channel, &session, "first", None, tx1)
            .await
            .expect("first");
        assert_eq!(text1, "first-done");

        let (tx2, _rx2) = tokio::sync::mpsc::channel(32);
        let text2 = runtime
            .process_with_progress(&channel, &session, "second", None, tx2)
            .await
            .expect("second");
        assert_eq!(text2, "second-done");
    }

    #[tokio::test]
    async fn process_with_progress_errors_when_max_tool_rounds_exceeded() {
        let tool_call = ToolCall {
            id: "tc-prog-max".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"value": "x"}),
        };
        let llm = Arc::new(MockLlm::new(vec![ChatResponse::ToolCalls(
            vec![tool_call],
            TokenUsage::default(),
        )]));
        let memory = open_temp_memory().await;
        let runtime = AgentRuntime::new(
            llm,
            build_registry(),
            memory,
            "mock-provider",
            "mock-model",
            1,
        );
        let channel = ChannelId::from("cli:local");
        let session = SessionId::from("session-prog-max");
        let (tx, _rx) = tokio::sync::mpsc::channel(16);

        let err = runtime
            .process_with_progress(&channel, &session, "loop", None, tx)
            .await
            .expect_err("should exceed rounds");
        match err {
            proto::Error::Llm(LlmError::MaxToolRoundsExceeded) => {}
            other => panic!("unexpected error: {other}"),
        }
    }
}
