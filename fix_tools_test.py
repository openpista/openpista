import re

with open("crates/tools/src/container.rs", "r") as f:
    content = f.read()

pattern = r"""let \(cert, key\) = gateway::server::generate_self_signed_cert\(\)\.unwrap\(\);
            let mut server_crypto = rustls::ServerConfig::builder\(\)
                \.with_no_client_auth\(\)
                \.with_single_cert\(vec!\[cert\], key\)
                \.unwrap\(\);"""

replacement = r"""let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
            let cert_der = rustls::pki_types::CertificateDer::from(cert.cert.der().to_vec());
            let key_der = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
            let key = rustls::pki_types::PrivateKeyDer::Pkcs8(key_der);
            
            let mut server_crypto = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![cert_der], key)
                .unwrap();"""

content = re.sub(pattern, replacement, content)
with open("crates/tools/src/container.rs", "w") as f:
    f.write(content)
