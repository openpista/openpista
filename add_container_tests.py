import os

with open("crates/tools/src/container.rs", "r") as f:
    content = f.read()

new_tests = """
    #[test]
    fn required_arg_string_returns_value_or_err() {
        assert_eq!(required_arg_string(Some("val"), "field").unwrap(), "val");
        assert!(required_arg_string(Some(""), "field").is_err());
        assert!(required_arg_string(None, "field").is_err());
    }

    #[tokio::test]
    async fn submit_worker_report_over_quic_fails_invalid_addr() {
        let addr = "127.0.0.1:0".parse().unwrap();
        let event = proto::ChannelEvent::new(
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
        );
        let result = submit_worker_report_over_quic(addr, event, 1).await;
        // Since no server is listening, it should fail.
        assert!(result.is_err());
    }

    // Mocking an actual QUIC server to test success path
    #[tokio::test]
    async fn submit_worker_report_over_quic_success() {
        use proto::ChannelEvent;
        use std::net::SocketAddr;
        use tokio::net::UdpSocket;
        use std::sync::Arc;
        
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        
        // Spawn a very dumb QUIC server that accepts one connection, reads JSON, and writes a response.
        tokio::spawn(async move {
            let (cert, key) = gateway::server::generate_self_signed_cert().unwrap();
            let mut server_crypto = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert], key)
                .unwrap();
            server_crypto.alpn_protocols = vec![b"openpista-quic-v1".to_vec()];
            let server_config = quinn::ServerConfig::with_crypto(Arc::new(
                quinn::crypto::rustls::QuicServerConfig::try_from(server_crypto).unwrap()
            ));
            
            let endpoint = quinn::Endpoint::server(server_config, addr).unwrap();
            
            if let Some(incoming) = endpoint.accept().await {
                if let Ok(conn) = incoming.await {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        // Read len
                        let mut len_buf = [0u8; 4];
                        let _ = recv.read_exact(&mut len_buf).await;
                        // Write dummy response
                        let resp = b"{\"channel_id\": \"cli:test\", \"session_id\": \"ses\", \"content\": \"ok\", \"is_error\": false}";
                        let resp_len = (resp.len() as u32).to_be_bytes();
                        let _ = send.write_all(&resp_len).await;
                        let _ = send.write_all(resp).await;
                        let _ = send.finish();
                    }
                }
            }
        });
        
        let event = ChannelEvent::new(
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
        );
        
        let result = submit_worker_report_over_quic(addr, event, 5).await;
        assert!(result.is_ok(), "Expected success, got {:?}", result);
    }
"""

if "required_arg_string_returns_value_or_err" not in content:
    content = content.replace("mod tests {", "mod tests {" + new_tests)
    with open("crates/tools/src/container.rs", "w") as f:
        f.write(content)
