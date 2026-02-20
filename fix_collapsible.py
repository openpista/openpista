with open("crates/tools/src/container.rs", "r") as f:
    content = f.read()

old = """        tokio::spawn(async move {
            if let Some(incoming) = server_endpoint.accept().await {
                if let Ok(conn) = incoming.await {
                    if let Ok((mut send, mut recv)) = conn.accept_bi().await {
                        let mut len_buf = [0u8; 4];
                        let _ = recv.read_exact(&mut len_buf).await;
                        let len = u32::from_be_bytes(len_buf) as usize;
                        let mut body = vec![0u8; len];
                        let _ = recv.read_exact(&mut body).await;
                        let resp = serde_json::json!({
                            "channel_id": "cli:test",
                            "session_id": "ses",
                            "content": "ok",
                            "is_error": false
                        })
                        .to_string();
                        let resp_bytes = resp.as_bytes();
                        let resp_len = (resp_bytes.len() as u32).to_be_bytes();
                        let _ = send.write_all(&resp_len).await;
                        let _ = send.write_all(resp_bytes).await;
                        let _ = send.finish();
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                }
            }
        });"""

new = """        tokio::spawn(async move {
            if let Some(incoming) = server_endpoint.accept().await
                && let Ok(conn) = incoming.await
                && let Ok((mut send, mut recv)) = conn.accept_bi().await
            {
                let mut len_buf = [0u8; 4];
                let _ = recv.read_exact(&mut len_buf).await;
                let len = u32::from_be_bytes(len_buf) as usize;
                let mut body = vec![0u8; len];
                let _ = recv.read_exact(&mut body).await;
                let resp = serde_json::json!({
                    "channel_id": "cli:test",
                    "session_id": "ses",
                    "content": "ok",
                    "is_error": false
                })
                .to_string();
                let resp_bytes = resp.as_bytes();
                let resp_len = (resp_bytes.len() as u32).to_be_bytes();
                let _ = send.write_all(&resp_len).await;
                let _ = send.write_all(resp_bytes).await;
                let _ = send.finish();
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        });"""

content = content.replace(old, new)
with open("crates/tools/src/container.rs", "w") as f:
    f.write(content)
