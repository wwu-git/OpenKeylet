use super::*;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use std::{fs, path::Path, process::Command};

const NOW: i64 = 1_800_000_000;
fn ca_params() -> CertificateParams {
    let mut p = CertificateParams::default();
    p.distinguished_name = DistinguishedName::new();
    p.distinguished_name
        .push(DnType::CommonName, "Synthetic test CA");
    p.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    p.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    p.not_before = ::time::OffsetDateTime::from_unix_timestamp(NOW - 100).unwrap();
    p.not_after = ::time::OffsetDateTime::from_unix_timestamp(NOW + 10000).unwrap();
    p
}
fn ca_fixture() -> (String, String) {
    let key = KeyPair::generate().unwrap();
    let cert = ca_params().self_signed(&key).unwrap();
    (key.serialize_pem(), cert.pem())
}
fn material<'a>(key: &'a str, cert: Option<&'a str>) -> IssuerMaterial<'a> {
    IssuerMaterial {
        private_key: key,
        passphrase: None,
        ca_certificate: cert,
    }
}
fn csr_fixture() -> (String, String) {
    let key = KeyPair::generate().unwrap();
    let p = CertificateParams::new(vec!["ignored.invalid".into()]).unwrap();
    (
        key.serialize_pem(),
        p.serialize_request(&key).unwrap().pem().unwrap(),
    )
}
fn tls_request(csr: String, kind: CertificateKind) -> CertificateRequest {
    if kind == CertificateKind::TlsServer {
        CertificateRequest::TlsServer {
            csr,
            names: vec!["server.test".into(), "127.0.0.1".into()],
        }
    } else {
        CertificateRequest::TlsClient {
            csr,
            names: vec!["client.test".into()],
        }
    }
}
fn verify_input(cert: &str, ca: &str, kind: CertificateKind) -> VerifyRequest {
    VerifyRequest {
        certificate: cert.into(),
        authorities: vec![ca.into()],
        intermediates: vec![],
        identity: if kind == CertificateKind::TlsClient {
            "client.test"
        } else {
            "server.test"
        }
        .into(),
        kind,
    }
}
fn tool(program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .output()
        .expect("independent verification tool must be installed");
    assert!(
        output.status.success(),
        "independent verification tool failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}
fn write(path: &Path, value: &str) {
    fs::write(path, value).unwrap();
}
fn ssh_keygen() -> &'static str {
    if Path::new("/usr/bin/ssh-keygen").exists() {
        "/usr/bin/ssh-keygen"
    } else {
        "ssh-keygen"
    }
}
fn openssl() -> &'static str {
    for path in [
        "/usr/local/opt/openssl@3/bin/openssl",
        "/opt/homebrew/opt/openssl@3/bin/openssl",
    ] {
        if Path::new(path).exists() {
            return path;
        }
    }
    "openssl"
}

#[test]
fn tls_issuance_verifies_name_purpose_time_and_key_match() {
    let (ca, cert) = ca_fixture();
    let (subject, csr) = csr_fixture();
    let mat = material(&ca, Some(&cert));
    let identity = issuer_identity(&mat, CertificateKind::TlsServer, NOW).unwrap();
    assert!(identity.certificate_fingerprint.is_some());
    assert_eq!(inspect_certificate(&cert).unwrap().kind, None);
    assert!(inspect_certificate(&cert).unwrap().is_ca);
    for kind in [CertificateKind::TlsServer, CertificateKind::TlsClient] {
        let issued =
            issue_certificate(&mat, &tls_request(csr.clone(), kind), 3600, &[1], NOW).unwrap();
        let mut req = verify_input(&issued.certificate, &cert, kind);
        let report = verify_certificate(&req, NOW).unwrap();
        assert!(report.verified, "{:?}", report.reasons);
        assert!(!report.revocation_checked);
        assert!(
            check_key(&subject, None, &issued.certificate)
                .unwrap()
                .matches
        );
        req.identity = "wrong.test".into();
        let report = verify_certificate(&req, NOW).unwrap();
        assert!(!report.verified);
        assert!(!report.identity_matches);
        req = verify_input(&issued.certificate, &cert, kind);
        req.kind = if kind == CertificateKind::TlsServer {
            CertificateKind::TlsClient
        } else {
            CertificateKind::TlsServer
        };
        assert!(!verify_certificate(&req, NOW).unwrap().verified);
        req = verify_input(&issued.certificate, &cert, kind);
        assert!(!verify_certificate(&req, NOW + 3600).unwrap().verified);
        assert!(!verify_certificate(&req, NOW - 1).unwrap().verified);
        let (_, other) = ca_fixture();
        req.authorities = vec![other];
        let report = verify_certificate(&req, NOW).unwrap();
        assert!(!report.verified);
        assert!(!report.issuer_trusted);
    }
}

#[test]
fn tls_rejects_bad_issuer_and_outliving_validity() {
    let (key, cert) = ca_fixture();
    let (_, csr) = csr_fixture();
    let req = tls_request(csr, CertificateKind::TlsServer);
    let combined = format!("{cert}\n{key}");
    assert!(issue_certificate(&material(&key, Some(&combined)), &req, 60, &[1], NOW).is_err());
    let (wrong, _) = ca_fixture();
    assert!(issue_certificate(&material(&wrong, Some(&cert)), &req, 60, &[1], NOW).is_err());
    assert!(issue_certificate(&material(&key, None), &req, 60, &[1], NOW).is_err());
    assert!(issue_certificate(&material(&key, Some(&cert)), &req, 10001, &[1], NOW).is_err());
    assert!(
        issuer_identity(
            &material(&key, Some(&cert)),
            CertificateKind::TlsServer,
            NOW + 10001
        )
        .is_err()
    );
    let kp = KeyPair::from_pem(&key).unwrap();
    for mut params in [CertificateParams::default(), {
        let mut p = ca_params();
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p
    }] {
        params.not_before = ::time::OffsetDateTime::from_unix_timestamp(NOW - 1).unwrap();
        params.not_after = ::time::OffsetDateTime::from_unix_timestamp(NOW + 100).unwrap();
        let bad = params.self_signed(&kp).unwrap().pem();
        assert!(
            issuer_identity(&material(&key, Some(&bad)), CertificateKind::TlsServer, NOW).is_err()
        );
    }
    let mut p = ca_params();
    p.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
    let bad = p.self_signed(&kp).unwrap().pem();
    assert!(issue_certificate(&material(&key, Some(&bad)), &req, 60, &[1], NOW).is_err());
}

#[test]
fn tls_csr_signature_checked_and_privilege_extensions_not_copied() {
    let (key, ca) = ca_fixture();
    let kp = KeyPair::generate().unwrap();
    let mut p = ca_params();
    p.subject_alt_names = CertificateParams::new(vec!["evil.invalid".into()])
        .unwrap()
        .subject_alt_names;
    p.extended_key_usages = vec![
        ExtendedKeyUsagePurpose::Any,
        ExtendedKeyUsagePurpose::CodeSigning,
    ];
    let csr = p.serialize_request(&kp).unwrap();
    let issued = issue_certificate(
        &material(&key, Some(&ca)),
        &tls_request(csr.pem().unwrap(), CertificateKind::TlsServer),
        60,
        &[1],
        NOW,
    )
    .unwrap();
    let summary = inspect_certificate(&issued.certificate).unwrap();
    assert!(!summary.is_ca);
    assert_eq!(summary.kind, Some(CertificateKind::TlsServer));
    assert!(!summary.identities.iter().any(|s| s == "evil.invalid"));
    let mut der = csr.der().to_vec();
    *der.last_mut().unwrap() ^= 1;
    let corrupted = format!(
        "-----BEGIN CERTIFICATE REQUEST-----\n{}\n-----END CERTIFICATE REQUEST-----",
        base64::engine::general_purpose::STANDARD.encode(der)
    );
    assert!(
        issue_certificate(
            &material(&key, Some(&ca)),
            &tls_request(corrupted, CertificateKind::TlsServer),
            60,
            &[2],
            NOW
        )
        .is_err()
    );
}

#[test]
fn tls_intermediate_chain_requires_explicit_trusted_root() {
    let (root_key, root) = ca_fixture();
    let issuer = Issuer::from_ca_cert_pem(&root, KeyPair::from_pem(&root_key).unwrap()).unwrap();
    let intermediate_key = KeyPair::generate().unwrap();
    let mut p = ca_params();
    p.distinguished_name
        .push(DnType::CommonName, "Synthetic intermediate");
    let intermediate = p.signed_by(&intermediate_key, &issuer).unwrap().pem();
    let (_, csr) = csr_fixture();
    let issued = issue_certificate(
        &material(&intermediate_key.serialize_pem(), Some(&intermediate)),
        &tls_request(csr, CertificateKind::TlsServer),
        60,
        &[3],
        NOW,
    )
    .unwrap();
    let mut req = verify_input(&issued.certificate, &root, CertificateKind::TlsServer);
    assert!(!verify_certificate(&req, NOW).unwrap().verified);
    req.intermediates = vec![intermediate];
    assert!(verify_certificate(&req, NOW).unwrap().verified);
}

#[test]
fn tls_openssl_independently_verifies_issued_server_and_client() {
    let dir = tempfile::tempdir().unwrap();
    let (key, ca) = ca_fixture();
    let (_, csr) = csr_fixture();
    write(&dir.path().join("ca.pem"), &ca);
    let csr_path = dir.path().join("request.pem");
    write(&csr_path, &csr);
    tool(
        openssl(),
        &[
            "req",
            "-in",
            csr_path.to_str().unwrap(),
            "-verify",
            "-noout",
        ],
    );
    for (kind, purpose) in [
        (CertificateKind::TlsServer, "sslserver"),
        (CertificateKind::TlsClient, "sslclient"),
    ] {
        let issued = issue_certificate(
            &material(&key, Some(&ca)),
            &tls_request(csr.clone(), kind),
            600,
            &[4],
            NOW,
        )
        .unwrap();
        let leaf = dir.path().join("leaf.pem");
        write(&leaf, &issued.certificate);
        let out = tool(
            openssl(),
            &[
                "verify",
                "-attime",
                &NOW.to_string(),
                "-CAfile",
                dir.path().join("ca.pem").to_str().unwrap(),
                "-purpose",
                purpose,
                leaf.to_str().unwrap(),
            ],
        );
        assert!(out.contains(": OK"));
        let wrong = Command::new(openssl())
            .args([
                "verify",
                "-attime",
                &NOW.to_string(),
                "-CAfile",
                dir.path().join("ca.pem").to_str().unwrap(),
                "-verify_hostname",
                "wrong.test",
                leaf.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!wrong.status.success());
        let wrong = Command::new(openssl())
            .args([
                "verify",
                "-attime",
                &NOW.to_string(),
                "-CAfile",
                dir.path().join("ca.pem").to_str().unwrap(),
                "-purpose",
                if purpose == "sslserver" {
                    "sslclient"
                } else {
                    "sslserver"
                },
                leaf.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!wrong.status.success());
    }
}

#[test]
fn tls_openssl_key_formats_and_encryption() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key.pem");
    let public = dir.path().join("public.pem");
    for args in [
        vec![
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
        ],
        vec![
            "genpkey",
            "-algorithm",
            "EC",
            "-pkeyopt",
            "ec_paramgen_curve:P-384",
        ],
        vec!["genpkey", "-algorithm", "ED25519"],
        vec!["ecparam", "-name", "prime256v1", "-genkey", "-noout"],
        vec!["genrsa", "-traditional", "2048"],
    ] {
        let mut cmd = Command::new(openssl());
        let output = cmd.args(args).output().unwrap();
        assert!(output.status.success());
        fs::write(&key, output.stdout).unwrap();
        tool(
            openssl(),
            &[
                "pkey",
                "-in",
                key.to_str().unwrap(),
                "-pubout",
                "-out",
                public.to_str().unwrap(),
            ],
        );
        let private = fs::read_to_string(&key).unwrap();
        let pubkey = fs::read_to_string(&public).unwrap();
        assert!(check_key(&private, None, &pubkey).unwrap().matches);
        let encrypted = dir.path().join("encrypted.pem");
        tool(
            openssl(),
            &[
                "pkcs8",
                "-topk8",
                "-in",
                key.to_str().unwrap(),
                "-out",
                encrypted.to_str().unwrap(),
                "-v2",
                "aes-256-cbc",
                "-passout",
                "pass:synthetic-test-only",
            ],
        );
        let encrypted = fs::read_to_string(encrypted).unwrap();
        assert!(
            check_key(&encrypted, Some("synthetic-test-only"), &pubkey)
                .unwrap()
                .matches
        );
        assert!(check_key(&encrypted, Some("wrong"), &pubkey).is_err());
    }
}

fn ssh_fixture(dir: &Path, name: &str, password: &str) -> (String, String) {
    let path = dir.join(name);
    tool(
        ssh_keygen(),
        &[
            "-q",
            "-t",
            "ed25519",
            "-N",
            password,
            "-f",
            path.to_str().unwrap(),
        ],
    );
    (
        fs::read_to_string(&path).unwrap(),
        fs::read_to_string(path.with_extension("pub")).unwrap(),
    )
}
#[test]
fn ssh_issuance_principals_extensions_time_and_trust() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, ca_pub) = ssh_fixture(dir.path(), "ca", "");
    let (subject, public) = ssh_fixture(dir.path(), "subject", "");
    let (_, other) = ssh_fixture(dir.path(), "other", "");
    for kind in [CertificateKind::SshUser, CertificateKind::SshHost] {
        let request = if kind == CertificateKind::SshUser {
            CertificateRequest::SshUser {
                public_key: public.clone(),
                principals: vec!["alice".into()],
                key_id: "synthetic-user".into(),
                extensions: vec!["permit-pty".into()],
            }
        } else {
            CertificateRequest::SshHost {
                public_key: public.clone(),
                principals: vec!["server.test".into()],
                key_id: "synthetic-host".into(),
            }
        };
        let issued = issue_certificate(&material(&ca, None), &request, 60, &[6], NOW).unwrap();
        assert!(
            check_key(&subject, None, &issued.certificate)
                .unwrap()
                .matches
        );
        let mut req = VerifyRequest {
            certificate: issued.certificate.clone(),
            authorities: vec![ca_pub.clone()],
            intermediates: vec![],
            identity: if kind == CertificateKind::SshUser {
                "alice"
            } else {
                "SERVER.test"
            }
            .into(),
            kind,
        };
        assert!(verify_certificate(&req, NOW).unwrap().verified);
        assert!(!verify_certificate(&req, NOW + 60).unwrap().verified);
        assert!(!verify_certificate(&req, NOW - 1).unwrap().verified);
        req.identity = "wrong".into();
        assert!(!verify_certificate(&req, NOW).unwrap().verified);
        req.authorities = vec![other.clone()];
        assert!(!verify_certificate(&req, NOW).unwrap().issuer_trusted);
        let path = dir.path().join("issued.pub");
        write(&path, &issued.certificate);
        let out = tool(ssh_keygen(), &["-L", "-f", path.to_str().unwrap()]);
        assert!(out.contains(if kind == CertificateKind::SshUser {
            "user certificate"
        } else {
            "host certificate"
        }));
        assert!(out.contains(if kind == CertificateKind::SshUser {
            "permit-pty"
        } else {
            "server.test"
        }));
    }
}

#[test]
fn ssh_encrypted_private_and_fail_closed_requests() {
    let dir = tempfile::tempdir().unwrap();
    let (ca, ca_pub) = ssh_fixture(dir.path(), "ca", "synthetic-test-only");
    let (_, public) = ssh_fixture(dir.path(), "subject", "");
    assert!(
        check_key(&ca, Some("synthetic-test-only"), &ca_pub)
            .unwrap()
            .matches
    );
    assert!(check_key(&ca, Some("wrong"), &ca_pub).is_err());
    let mat = IssuerMaterial {
        private_key: &ca,
        passphrase: Some("synthetic-test-only"),
        ca_certificate: None,
    };
    let mut req = CertificateRequest::SshUser {
        public_key: public,
        principals: vec![],
        key_id: "test".into(),
        extensions: vec![],
    };
    assert!(issue_certificate(&mat, &req, 60, &[1], NOW).is_err());
    if let CertificateRequest::SshUser {
        principals,
        extensions,
        ..
    } = &mut req
    {
        principals.push("alice".into());
        extensions.push("unknown-privilege".into());
    }
    assert!(issue_certificate(&mat, &req, 60, &[1], NOW).is_err());
    assert!(issue_certificate(&mat, &req, 0, &[1], NOW).is_err());
    assert!(issue_certificate(&mat, &req, 60, &[0], NOW).is_err());
    assert!(serde_json::from_str::<CertificateRequest>(r#"{"kind":"sshHost","publicKey":"test","principals":["test"],"keyId":"test","criticalOptions":[]}"#).is_err());
}

#[test]
fn key_match_is_not_certificate_trust_and_rejects_different_public_key() {
    let (ca, cert) = ca_fixture();
    let (subject, csr) = csr_fixture();
    let wrong = KeyPair::generate().unwrap();
    let public = KeyPair::from_pem(&subject).unwrap().public_key_pem();
    assert!(check_key(&subject, None, &public).unwrap().matches);
    assert!(
        !check_key(&wrong.serialize_pem(), None, &public)
            .unwrap()
            .matches
    );
    let issued = issue_certificate(
        &material(&ca, Some(&cert)),
        &tls_request(csr, CertificateKind::TlsServer),
        60,
        &[5],
        NOW,
    )
    .unwrap();
    assert!(
        !check_key(&wrong.serialize_pem(), None, &issued.certificate)
            .unwrap()
            .matches
    );
    let dir = tempfile::tempdir().unwrap();
    let (ssh, ssh_pub) = ssh_fixture(dir.path(), "one", "");
    let (_, other_pub) = ssh_fixture(dir.path(), "two", "");
    assert!(check_key(&ssh, None, &ssh_pub).unwrap().matches);
    assert!(!check_key(&ssh, None, &other_pub).unwrap().matches);
}

#[test]
fn ssh_unknown_critical_options_and_wrong_host_type_fail_closed() {
    use russh::keys::{
        decode_secret_key,
        ssh_key::{
            HashAlg, PublicKey,
            certificate::{Builder, CertType},
        },
    };
    let dir = tempfile::tempdir().unwrap();
    let (ca, ca_pub) = ssh_fixture(dir.path(), "ca", "");
    let (_, public) = ssh_fixture(dir.path(), "subject", "");
    let ca = decode_secret_key(&ca, None).unwrap();
    let public = PublicKey::from_openssh(&public).unwrap();
    for kind in [CertType::User, CertType::Host] {
        let mut builder = Builder::new(
            vec![7; 32],
            public.key_data().clone(),
            NOW as u64,
            (NOW + 60) as u64,
        )
        .unwrap();
        builder
            .cert_type(kind)
            .unwrap()
            .key_id("synthetic")
            .unwrap()
            .serial(1)
            .unwrap()
            .valid_principal("server.test")
            .unwrap()
            .critical_option("source-address", "127.0.0.1")
            .unwrap();
        let cert = builder.sign(&ca).unwrap();
        let req = VerifyRequest {
            certificate: cert.to_openssh().unwrap(),
            authorities: vec![ca_pub.clone()],
            intermediates: vec![],
            identity: "server.test".into(),
            kind: CertificateKind::SshHost,
        };
        let report = verify_certificate(&req, NOW).unwrap();
        assert!(report.signature_valid);
        assert!(!report.purpose_valid);
        assert!(!report.verified);
        assert!(
            verify_ssh_host_certificate(
                &cert,
                "server.test",
                &ca.public_key().fingerprint(HashAlg::Sha256).to_string(),
                NOW
            )
            .is_err()
        );
    }
}

#[test]
fn tls_ca_name_constraints_and_weak_keys_fail_closed() {
    use rcgen::{GeneralSubtree, NameConstraints};
    let key = KeyPair::generate().unwrap();
    let mut p = ca_params();
    p.name_constraints = Some(NameConstraints {
        permitted_subtrees: vec![GeneralSubtree::DnsName("allowed.test".into())],
        excluded_subtrees: vec![],
    });
    let ca = p.self_signed(&key).unwrap().pem();
    let (_, csr) = csr_fixture();
    assert!(
        issue_certificate(
            &material(&key.serialize_pem(), Some(&ca)),
            &tls_request(csr, CertificateKind::TlsServer),
            60,
            &[1],
            NOW
        )
        .is_err()
    );
    let output = Command::new(openssl())
        .args([
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:1024",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let weak = String::from_utf8(output.stdout).unwrap();
    assert!(check_key(&weak, None, &key.public_key_pem()).is_err());
    let mut request = tls_request("not a CSR".into(), CertificateKind::TlsServer);
    assert!(
        issue_certificate(
            &material(&key.serialize_pem(), Some(&ca)),
            &request,
            60,
            &[1],
            NOW
        )
        .is_err()
    );
    if let CertificateRequest::TlsServer { names, .. } = &mut request {
        names.clear();
    }
    assert!(
        issue_certificate(
            &material(&key.serialize_pem(), Some(&ca)),
            &request,
            60,
            &[1],
            NOW
        )
        .is_err()
    );
}

struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_child(mut child: std::process::Child) -> std::process::Output {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if child.try_wait().unwrap().is_some() {
            return child.wait_with_output().unwrap();
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("synthetic TLS client exceeded deadline");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn tls_real_local_handshake_accepts_trust_and_name_rejects_wrong_name_and_ca() {
    use std::{
        net::{TcpListener, TcpStream},
        process::Stdio,
        time::{Duration, Instant},
    };
    let dir = tempfile::tempdir().unwrap();
    let (ca_key, ca) = ca_fixture();
    let (_, other) = ca_fixture();
    let (subject, csr) = csr_fixture();
    let issued = issue_certificate(
        &material(&ca_key, Some(&ca)),
        &tls_request(csr, CertificateKind::TlsServer),
        600,
        &[99],
        NOW,
    )
    .unwrap();
    for (name, value) in [
        ("ca.pem", ca),
        ("other.pem", other),
        ("leaf.pem", issued.certificate),
        ("key.pem", subject),
    ] {
        write(&dir.path().join(name), &value);
    }
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("loopback TLS acceptance requires local bind");
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let address = format!("127.0.0.1:{port}");
    let server = Command::new(openssl())
        .args([
            "s_server",
            "-accept",
            &address,
            "-cert",
            dir.path().join("leaf.pem").to_str().unwrap(),
            "-key",
            dir.path().join("key.pem").to_str().unwrap(),
            "-quiet",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut server = ChildGuard(server);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if TcpStream::connect(&address).is_ok() {
            break;
        }
        assert!(
            server.0.try_wait().unwrap().is_none(),
            "synthetic TLS server exited before readiness"
        );
        assert!(
            Instant::now() < deadline,
            "synthetic TLS server readiness deadline"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for (authority, name, success) in [
        ("ca.pem", "server.test", true),
        ("ca.pem", "wrong.test", false),
        ("other.pem", "server.test", false),
    ] {
        let child = Command::new(openssl())
            .args([
                "s_client",
                "-connect",
                &address,
                "-CAfile",
                dir.path().join(authority).to_str().unwrap(),
                "-verify_hostname",
                name,
                "-verify_return_error",
                "-attime",
                &NOW.to_string(),
                "-brief",
                "-no_ign_eof",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = wait_child(child);
        assert_eq!(
            output.status.success(),
            success,
            "synthetic TLS handshake: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if success {
            assert!(String::from_utf8_lossy(&output.stderr).contains("Verification: OK"));
        }
    }
}

#[test]
fn ssh_encrypted_pkcs8_matches_openssh_public_key() {
    let dir = tempfile::tempdir().unwrap();
    let key = dir.path().join("key.pem");
    let public = dir.path().join("public.pem");
    let encrypted = dir.path().join("encrypted.pem");
    tool(
        openssl(),
        &[
            "genpkey",
            "-algorithm",
            "RSA",
            "-pkeyopt",
            "rsa_keygen_bits:2048",
            "-out",
            key.to_str().unwrap(),
        ],
    );
    tool(
        openssl(),
        &[
            "pkey",
            "-in",
            key.to_str().unwrap(),
            "-pubout",
            "-out",
            public.to_str().unwrap(),
        ],
    );
    let ssh_public = tool(
        ssh_keygen(),
        &["-i", "-m", "PKCS8", "-f", public.to_str().unwrap()],
    );
    tool(
        openssl(),
        &[
            "pkcs8",
            "-topk8",
            "-in",
            key.to_str().unwrap(),
            "-out",
            encrypted.to_str().unwrap(),
            "-v2",
            "aes-256-cbc",
            "-passout",
            "pass:synthetic-test-only",
        ],
    );
    assert!(
        check_key(
            &fs::read_to_string(encrypted).unwrap(),
            Some("synthetic-test-only"),
            &ssh_public
        )
        .unwrap()
        .matches
    );
}

#[test]
fn encrypted_pkcs8_rejects_excessive_kdf_work_before_decryption() {
    use pkcs8::{der::asn1::OctetStringRef, pkcs5::pbes2::Parameters};
    let params =
        Parameters::generate_pbkdf2_sha256_aes256cbc(2_000_001, &[1; 16], [2; 16]).unwrap();
    let encrypted = pkcs8::EncryptedPrivateKeyInfo {
        encryption_algorithm: params.into(),
        encrypted_data: OctetStringRef::new(&[0; 16]).unwrap(),
    };
    let document = pkcs8::SecretDocument::try_from(&encrypted).unwrap();
    let pem = document
        .to_pem("ENCRYPTED PRIVATE KEY", pkcs8::LineEnding::LF)
        .unwrap();
    let error = super::tls::decrypt_pkcs8(&pem, Some("synthetic-test-only")).unwrap_err();
    assert_eq!(error.to_string(), "Unsupported encrypted key work factor");
}

#[test]
fn ssh_host_verification_rejects_weak_ca_even_when_pinned() {
    use russh::keys::{
        decode_secret_key,
        ssh_key::{
            HashAlg, PublicKey,
            certificate::{Builder, CertType},
        },
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("weak-ca");
    tool(
        ssh_keygen(),
        &[
            "-q",
            "-t",
            "rsa",
            "-b",
            "1024",
            "-N",
            "",
            "-f",
            path.to_str().unwrap(),
        ],
    );
    let weak = decode_secret_key(&fs::read_to_string(path).unwrap(), None).unwrap();
    let (_, public) = ssh_fixture(dir.path(), "subject", "");
    let public = PublicKey::from_openssh(&public).unwrap();
    let mut builder = Builder::new(
        vec![9; 32],
        public.key_data().clone(),
        NOW as u64,
        (NOW + 60) as u64,
    )
    .unwrap();
    builder
        .cert_type(CertType::Host)
        .unwrap()
        .serial(1)
        .unwrap()
        .key_id("synthetic weak CA")
        .unwrap()
        .valid_principal("server.test")
        .unwrap();
    let cert = builder.sign(&weak).unwrap();
    assert!(cert.verify_signature().is_ok());
    let fingerprint = weak.public_key().fingerprint(HashAlg::Sha256).to_string();
    let error = verify_ssh_host_certificate(&cert, "server.test", &fingerprint, NOW).unwrap_err();
    assert_eq!(error.to_string(), "Unsupported or weak SSH key algorithm");
}
