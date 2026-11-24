use super::*;
use ssh_key::certificate::{Builder, CertType};

fn signed(
    subject: u8,
    issuer: u8,
    kind: CertType,
    principal: &str,
    start: u64,
    end: u64,
) -> ssh_key::Certificate {
    let mut builder = Builder::new(
        vec![7; 32],
        key(subject).public_key().key_data().clone(),
        start,
        end,
    )
    .unwrap();
    builder.cert_type(kind).unwrap();
    builder.valid_principal(principal).unwrap();
    builder.key_id("synthetic certificate fixture").unwrap();
    builder.sign(&key(issuer)).unwrap()
}
fn user(subject: u8) -> ssh_key::Certificate {
    let now = chrono::Utc::now().timestamp() as u64;
    signed(
        subject,
        10,
        CertType::User,
        "service-principal",
        now - 60,
        now + 3600,
    )
}
fn host(subject: u8, principal: &str) -> ssh_key::Certificate {
    let now = chrono::Utc::now().timestamp() as u64;
    signed(subject, 11, CertType::Host, principal, now - 60, now + 3600)
}
fn with_critical_option(subject: u8, kind: CertType, principal: &str) -> ssh_key::Certificate {
    let now = chrono::Utc::now().timestamp() as u64;
    let mut builder = Builder::new(
        vec![7; 32],
        key(subject).public_key().key_data().clone(),
        now - 60,
        now + 3600,
    )
    .unwrap();
    builder.cert_type(kind).unwrap();
    builder.valid_principal(principal).unwrap();
    builder
        .critical_option("unknown-critical-policy", "required")
        .unwrap();
    builder
        .sign(&key(if kind == CertType::Host { 11 } else { 10 }))
        .unwrap()
}
fn tampered_user() -> ssh_key::Certificate {
    let mut bytes = user(3).to_bytes().unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    ssh_key::Certificate::from_bytes(&bytes).unwrap()
}
async fn certified(
    route: bool,
    a_host: Option<ssh_key::Certificate>,
    b_host: Option<ssh_key::Certificate>,
    b_user: ssh_key::Certificate,
) -> Fixture {
    let (b_port, b, b_task) =
        start_with_certificates(2, None, true, b_host, Some(key(10).public_key().clone())).await;
    let (a_port, a, a_task) = start_with_certificates(
        1,
        Some(b_port),
        true,
        a_host,
        Some(key(10).public_key().clone()),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().join("vault")).unwrap();
    let create = |name: &str,
                  hostname: &str,
                  port: u16,
                  username: &str,
                  private: u8,
                  certificate: ssh_key::Certificate,
                  jump: Option<Uuid>| {
        let mut value = input(name, hostname, port, username);
        value.ssh_auth_type = "certificate".into();
        value.ssh_jump_item_id = jump;
        value.modules.retain(|module| module.kind != "password");
        value
            .modules
            .push(serde_json::from_value(serde_json::json!({"kind":"privateKey"})).unwrap());
        value.modules.push(serde_json::from_value(serde_json::json!({"kind":"sshCertificate","value":certificate.to_openssh().unwrap()})).unwrap());
        value.secrets.password = None;
        let pem = key(private)
            .to_openssh(ssh_key::LineEnding::LF)
            .unwrap()
            .to_string();
        value.secrets.private_key = Some(pem.clone());
        vault
            .save_connection_with_private_key(
                value,
                None,
                Some((&pem, "synthetic certificate key")),
            )
            .unwrap()
            .id
    };
    let jump = create(
        "Certificate jump A",
        "127.0.0.1",
        a_port,
        "jump-user",
        4,
        user(4),
        None,
    );
    let target_host = if route {
        "private.test.invalid"
    } else {
        "127.0.0.1"
    };
    let target = create(
        "Certificate target B",
        target_host,
        b_port,
        "target-user",
        3,
        b_user,
        route.then_some(jump),
    );
    let ca = key(11)
        .public_key()
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string();
    vault
        .set_ssh_host_ca_trust(jump, "127.0.0.1", a_port, Some(&ca))
        .unwrap();
    vault
        .set_ssh_host_ca_trust(target, target_host, b_port, Some(&ca))
        .unwrap();
    Fixture {
        _dir: dir,
        vault,
        target,
        jump,
        a,
        b,
        a_port,
        b_port,
        tasks: vec![a_task, b_task],
    }
}
async fn valid(route: bool) -> Fixture {
    certified(
        route,
        Some(host(1, "127.0.0.1")),
        Some(host(
            2,
            if route {
                "private.test.invalid"
            } else {
                "127.0.0.1"
            },
        )),
        user(3),
    )
    .await
}

#[tokio::test]
async fn certificate_authentication_and_host_verification_work_direct_and_through_jump() {
    for route in [false, true] {
        let fixture = valid(route).await;
        let target = fixture.vault.get_connection(fixture.target).unwrap();
        assert_eq!(target.stored.ssh_auth_type, "certificate");
        assert_eq!(
            executor::test_connection(&fixture.vault, &target)
                .await
                .unwrap(),
            "SSH connection successful"
        );
        fixture.closed().await;
        assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
        assert_eq!(fixture.a.auth.load(Ordering::SeqCst) > 0, route);
        assert_eq!(fixture.a.forwards.lock().unwrap().is_empty(), !route);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn certificates_support_real_sftp_in_both_routes() {
    for route in [false, true] {
        let fixture = valid(route).await;
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("source");
        let remote = dir.path().join("remote");
        let downloaded = dir.path().join("downloaded");
        let content = vec![0xB7; 65_537];
        tokio::fs::write(&local, &content).await.unwrap();
        let target = fixture.vault.get_connection(fixture.target).unwrap();
        let upload = executor::ssh_upload(
            &fixture.vault,
            &target,
            local.to_str().unwrap(),
            remote.to_str().unwrap(),
            true,
            Some(10),
        )
        .await
        .unwrap();
        assert_eq!(upload.bytes_transferred, content.len() as u64);
        executor::ssh_download(
            &fixture.vault,
            &target,
            remote.to_str().unwrap(),
            downloaded.to_str().unwrap(),
            true,
            Some(10),
        )
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(&downloaded).await.unwrap(), content);
        fixture.closed().await;
        assert_eq!(fixture.a.auth.load(Ordering::SeqCst) > 0, route);
        assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
    }
}

#[tokio::test]
async fn invalid_host_certificates_stop_before_target_authentication_without_raw_key_fallback() {
    let now = chrono::Utc::now().timestamp() as u64;
    let bad = [
        Some(signed(
            2,
            12,
            CertType::Host,
            "private.test.invalid",
            now - 60,
            now + 3600,
        )),
        Some(host(2, "wrong.test.invalid")),
        Some(with_critical_option(
            2,
            CertType::Host,
            "private.test.invalid",
        )),
        Some(signed(
            2,
            11,
            CertType::Host,
            "private.test.invalid",
            now + 60,
            now + 3600,
        )),
        Some(signed(
            2,
            11,
            CertType::Host,
            "private.test.invalid",
            now - 120,
            now - 60,
        )),
        Some(signed(
            2,
            11,
            CertType::User,
            "private.test.invalid",
            now - 60,
            now + 3600,
        )),
        None,
    ];
    for certificate in bad {
        let fixture = certified(true, Some(host(1, "127.0.0.1")), certificate, user(3)).await;
        let error = fixture
            .command("hostname", Some(10))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("verification failed"), "{error}");
        assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
        assert_eq!(fixture.a.forwards.lock().unwrap().len(), 1);
        fixture.closed().await;
    }
}

#[tokio::test]
async fn invalid_jump_certificate_stops_before_either_authentication() {
    let fixture = certified(
        true,
        Some(host(1, "wrong.test.invalid")),
        Some(host(2, "private.test.invalid")),
        user(3),
    )
    .await;
    assert!(fixture.command("hostname", Some(10)).await.is_err());
    assert_eq!(fixture.a.auth.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
    assert!(fixture.a.forwards.lock().unwrap().is_empty());
    fixture.closed().await;
}

#[tokio::test]
async fn certificate_probe_returns_both_fingerprints_without_authenticating_or_trusting() {
    let fixture = valid(false).await;
    fixture
        .vault
        .set_ssh_host_ca_trust(fixture.target, "127.0.0.1", fixture.b_port, None)
        .unwrap();
    let result = probe(&fixture.vault, fixture.target).await.unwrap();
    assert_eq!(result.kind, SshHostTrustKind::CertificateAuthority);
    assert_eq!(
        result.fingerprint,
        key(2)
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string()
    );
    assert_eq!(
        result.ca_fingerprint,
        Some(
            key(11)
                .public_key()
                .fingerprint(ssh_key::HashAlg::Sha256)
                .to_string()
        )
    );
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
    assert!(
        fixture
            .vault
            .ssh_host_trust()
            .unwrap()
            .iter()
            .all(|entry| entry.item_id != fixture.target)
    );
    fixture.closed().await;
}

#[tokio::test]
async fn invalid_user_certificates_never_fall_back_to_ordinary_key_authentication() {
    let now = chrono::Utc::now().timestamp() as u64;
    let cases = [
        user(5),
        tampered_user(),
        with_critical_option(3, CertType::User, "service-principal"),
        signed(
            3,
            10,
            CertType::Host,
            "service-principal",
            now - 60,
            now + 3600,
        ),
        signed(
            3,
            10,
            CertType::User,
            "service-principal",
            now - 120,
            now - 60,
        ),
        signed(
            3,
            10,
            CertType::User,
            "service-principal",
            now + 60,
            now + 3600,
        ),
    ];
    for certificate in cases {
        let fixture = certified(
            false,
            Some(host(1, "127.0.0.1")),
            Some(host(2, "127.0.0.1")),
            certificate,
        )
        .await;
        assert!(fixture.command("hostname", Some(10)).await.is_err());
        assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
        fixture.closed().await;
    }
    let rejected_by_server = signed(
        3,
        12,
        CertType::User,
        "service-principal",
        now - 60,
        now + 3600,
    );
    let fixture = certified(
        false,
        Some(host(1, "127.0.0.1")),
        Some(host(2, "127.0.0.1")),
        rejected_by_server,
    )
    .await;
    assert!(fixture.command("hostname", Some(10)).await.is_err());
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 1);
    fixture.closed().await;
}

#[tokio::test]
async fn missing_duplicate_and_malformed_certificate_modules_fail_before_authentication() {
    for case in ["missing", "duplicate", "malformed"] {
        let fixture = valid(false).await;
        let mut target = fixture.vault.get_connection(fixture.target).unwrap();
        match case {
            "missing" => target
                .stored
                .modules
                .retain(|module| module.kind != "sshCertificate"),
            "duplicate" => {
                let certificate = target
                    .stored
                    .modules
                    .iter()
                    .find(|module| module.kind == "sshCertificate")
                    .unwrap()
                    .clone();
                target.stored.modules.push(certificate);
            }
            _ => {
                target
                    .stored
                    .modules
                    .iter_mut()
                    .find(|module| module.kind == "sshCertificate")
                    .unwrap()
                    .value = "invalid certificate".into()
            }
        }
        assert!(
            executor::test_connection(&fixture.vault, &target)
                .await
                .is_err(),
            "{case}"
        );
        assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0, "{case}");
        fixture.closed().await;
    }
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires a local OpenSSH sshd executable and OS account authorization"]
async fn local_openssh_accepts_generated_user_and_host_certificates() {
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let username = String::from_utf8(Command::new("id").arg("-un").output().unwrap().stdout)
        .unwrap()
        .trim()
        .to_owned();
    let now = chrono::Utc::now().timestamp() as u64;
    let user_cert = signed(
        3,
        10,
        CertType::User,
        "service-principal",
        now - 60,
        now + 3600,
    );
    for (name, bytes) in [
        (
            "host",
            key(2)
                .to_openssh(ssh_key::LineEnding::LF)
                .unwrap()
                .to_string(),
        ),
        ("host-cert.pub", host(2, "127.0.0.1").to_openssh().unwrap()),
        ("user-ca.pub", key(10).public_key().to_openssh().unwrap()),
        ("principals", "service-principal\n".into()),
    ] {
        std::fs::write(root.join(name), bytes).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(root.join("host"), std::fs::Permissions::from_mode(0o600)).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let configuration = format!(
        "Port {port}\nListenAddress 127.0.0.1\nHostKey {r}/host\nHostCertificate {r}/host-cert.pub\nTrustedUserCAKeys {r}/user-ca.pub\nAuthorizedPrincipalsFile {r}/principals\nAuthorizedKeysFile none\nStrictModes no\nPasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nPermitRootLogin yes\nPidFile {r}/sshd.pid\nLogLevel VERBOSE\nSubsystem sftp internal-sftp\n",
        r = root.display()
    );
    std::fs::write(root.join("sshd_config"), configuration).unwrap();
    let log = std::fs::File::create(root.join("sshd.log")).unwrap();
    let mut server = Command::new("/usr/sbin/sshd")
        .args(["-D", "-e", "-f"])
        .arg(root.join("sshd_config"))
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .unwrap();
    struct Stop(std::process::Child);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    for _ in 0..50 {
        if let Some(status) = server.try_wait().unwrap() {
            panic!(
                "local sshd exited {status}: {}",
                std::fs::read_to_string(root.join("sshd.log")).unwrap()
            );
        }
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _stop = Stop(server);
    let vault = Vault::open(root.join("vault")).unwrap();
    let mut value = input(
        "Generated OpenSSH interoperability fixture",
        "127.0.0.1",
        port,
        &username,
    );
    value.ssh_auth_type = "certificate".into();
    value.modules.retain(|module| module.kind != "password");
    value
        .modules
        .push(serde_json::from_value(serde_json::json!({"kind":"privateKey"})).unwrap());
    value.modules.push(
        serde_json::from_value(
            serde_json::json!({"kind":"sshCertificate","value":user_cert.to_openssh().unwrap()}),
        )
        .unwrap(),
    );
    value.secrets.password = None;
    let pem = key(3)
        .to_openssh(ssh_key::LineEnding::LF)
        .unwrap()
        .to_string();
    value.secrets.private_key = Some(pem.clone());
    let id = vault
        .save_connection_with_private_key(value, None, Some((&pem, "generated")))
        .unwrap()
        .id;
    let ca = key(11)
        .public_key()
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string();
    vault
        .set_ssh_host_ca_trust(id, "127.0.0.1", port, Some(&ca))
        .unwrap();
    let connection = vault.get_connection(id).unwrap();
    let result = executor::test_connection(&vault, &connection).await;
    assert!(
        result.is_ok(),
        "OpenSSH result: {result:?}; server: {}",
        std::fs::read_to_string(root.join("sshd.log")).unwrap()
    );
}
