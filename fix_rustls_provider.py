import re

with open("crates/tools/src/container.rs", "r") as f:
    content = f.read()

content = content.replace("async fn submit_worker_report_over_quic_fails_invalid_addr() {", 
                          "async fn submit_worker_report_over_quic_fails_invalid_addr() {\n        rustls::crypto::ring::default_provider().install_default().ok();")

content = content.replace("async fn submit_worker_report_over_quic_success() {",
                          "async fn submit_worker_report_over_quic_success() {\n        rustls::crypto::ring::default_provider().install_default().ok();")

with open("crates/tools/src/container.rs", "w") as f:
    f.write(content)
