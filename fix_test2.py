import re

with open("crates/tools/src/container.rs", "r") as f:
    content = f.read()

# Fix event creation
old_event = """        let event = proto::ChannelEvent::new(
            proto::ChannelId::new("cli", "test"),
            proto::SessionId::from("ses"),
            proto::WorkerReport::new(
                "call_1",
                "worker_1",
                "image",
                "cmd",
                proto::WorkerOutput {
                    exit_code: 0,
                    stdout: "".into(),
                    stderr: "".into(),
                    output: "".into(),
                }
            )
        );"""

new_event = """        let report = proto::WorkerReport::new(
            "call_1",
            "worker_1",
            "image",
            "cmd",
            proto::WorkerOutput {
                exit_code: 0,
                stdout: "".into(),
                stderr: "".into(),
                output: "".into(),
            }
        );
        let mut event = proto::ChannelEvent::new(
            proto::ChannelId::new("cli", "test"),
            proto::SessionId::from("ses"),
            "summary"
        );
        event.metadata = Some(serde_json::to_value(&report).unwrap());"""
content = content.replace(old_event, new_event)

old_event2 = """        let event = ChannelEvent::new(
            proto::ChannelId::new("cli", "test"),
            proto::SessionId::from("ses"),
            proto::WorkerReport::new(
                "call_1",
                "worker_1",
                "image",
                "cmd",
                proto::WorkerOutput {
                    exit_code: 0,
                    stdout: "".into(),
                    stderr: "".into(),
                    output: "".into(),
                }
            )
        );"""
content = content.replace(old_event2, new_event.replace("proto::ChannelEvent", "ChannelEvent"))

# Fix Duration
content = content.replace(", 1).await;", ", std::time::Duration::from_secs(1)).await;")
content = content.replace(", 5).await;", ", std::time::Duration::from_secs(5)).await;")

# Fix byte string
content = content.replace('let resp = b"{"channel_id": "cli:test", "session_id": "ses", "content": "ok", "is_error": false}";', 'let resp = br#"{"channel_id": "cli:test", "session_id": "ses", "content": "ok", "is_error": false}"#;')

with open("crates/tools/src/container.rs", "w") as f:
    f.write(content)
