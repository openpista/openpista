use proto::{
    AgentResponse, ChannelEvent, ChannelId, SessionId, WORKER_REPORT_KIND, WorkerOutput,
    WorkerReport,
};

#[test]
fn worker_report_metadata_round_trip_contract() {
    let report = WorkerReport::new(
        "call-42",
        "worker-1",
        "alpine:3.20",
        "echo ok",
        WorkerOutput {
            exit_code: 0,
            stdout: "ok\n".to_string(),
            stderr: String::new(),
            output: "stdout:\nok\n".to_string(),
        },
    );

    let mut event = ChannelEvent::new(
        ChannelId::new("cli", "local"),
        SessionId::from("session-a"),
        "run",
    );
    event.metadata = Some(serde_json::to_value(&report).expect("serialize report"));

    let metadata = event.metadata.expect("metadata should exist");
    let parsed: WorkerReport = serde_json::from_value(metadata).expect("parse worker report");
    assert_eq!(parsed.kind, WORKER_REPORT_KIND);
    assert_eq!(parsed.call_id, "call-42");
    assert!(parsed.is_worker_report());
}

#[test]
fn error_response_preserves_channel_and_session_contract() {
    let channel_id = ChannelId::new("telegram", "1001");
    let session_id = SessionId::from("s-err");
    let response = AgentResponse::error(channel_id.clone(), session_id.clone(), "bad token");

    assert!(response.is_error);
    assert_eq!(response.channel_id, channel_id);
    assert_eq!(response.session_id, session_id);
    assert_eq!(response.content, "bad token");
}
