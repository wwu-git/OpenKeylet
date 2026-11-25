use super::*;
use crate::{executor, model::ConnectionInput};
use russh::{
    Channel, ChannelId,
    server::{self, Server as _},
};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::atomic::{AtomicUsize, Ordering},
};

fn key(seed: u8) -> russh::keys::PrivateKey {
    russh::keys::PrivateKey::from(ssh_key::private::Ed25519Keypair::from_seed(&[seed; 32]))
}

#[derive(Clone)]
struct TestServer {
    auth: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    forwards: Arc<Mutex<Vec<(String, u32)>>>,
    forward_port: Option<u16>,
    allow_forward: bool,
    public: ssh_key::PublicKey,
    certificate_authority: Option<ssh_key::PublicKey>,
}
struct TestHandler {
    server: TestServer,
    channels: HashMap<ChannelId, Channel<server::Msg>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for TestHandler {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.server.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl server::Server for TestServer {
    type Handler = TestHandler;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self::Handler {
        self.active.fetch_add(1, Ordering::SeqCst);
        TestHandler {
            server: self.clone(),
            channels: HashMap::new(),
            tasks: vec![],
        }
    }
}
impl server::Handler for TestHandler {
    type Error = anyhow::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth> {
        self.server.auth.fetch_add(1, Ordering::SeqCst);
        Ok(
            if (self.server.forward_port.is_some()
                && user == "jump-user"
                && password == "jump-simulated-password")
                || (self.server.forward_port.is_none()
                    && user == "target-user"
                    && password == "target-simulated-password")
            {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn auth_publickey(
        &mut self,
        user: &str,
        public: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth> {
        let public = public.public_key;
        self.server.auth.fetch_add(1, Ordering::SeqCst);
        Ok(
            if (self.server.forward_port.is_none()
                && user == "target-user"
                && *public == self.server.public)
                || (self.server.forward_port.is_some()
                    && user == "jump-user"
                    && *public == *key(4).public_key())
            {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn auth_openssh_certificate(
        &mut self,
        user: &str,
        certificate: &ssh_key::Certificate,
    ) -> Result<server::Auth> {
        self.server.auth.fetch_add(1, Ordering::SeqCst);
        let expected_user = if self.server.forward_port.is_some() { "jump-user" } else { "target-user" };
        let expected_key = if self.server.forward_port.is_some() { key(4) } else { key(3) };
        let trusted = self.server.certificate_authority.as_ref().is_some_and(|ca| {
            let fingerprint = ca.fingerprint(ssh_key::HashAlg::Sha256);
            certificate.validate_at(chrono::Utc::now().timestamp() as u64, [&fingerprint]).is_ok()
        });
        Ok(if trusted && user == expected_user
            && certificate.cert_type() == ssh_key::certificate::CertType::User
            && certificate.public_key() == expected_key.public_key().key_data()
            && certificate.valid_principals().iter().any(|principal| principal == "service-principal")
            && certificate.critical_options().is_empty()
        { server::Auth::Accept } else { server::Auth::reject() })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<()> {
        self.server
            .forwards
            .lock()
            .unwrap()
            .push((host.into(), port));
        if !self.server.allow_forward {
            return Ok(());
        }
        ensure!(
            host == "private.test.invalid",
            "Unexpected forwarded destination"
        );
        let mut tcp = tokio::net::TcpStream::connect((
            "127.0.0.1",
            self.server.forward_port.context("No forwarding target")?,
        ))
        .await?;
        reply.accept().await;
        self.tasks.push(tokio::spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
        }));
        Ok(())
    }
    async fn channel_open_session(
        &mut self,
        channel: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<()> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut server::Session,
    ) -> Result<()> {
        session.channel_success(channel)?;
        if command != b"hold" {
            let output = if command == b"echo mcp-vault-ok" {
                b"mcp-vault-ok\r\n".to_vec()
            } else if self.server.forward_port.is_some() {
                b"jump-A".to_vec()
            } else if command == b"redaction" {
                b"jumpsecret jump anVtcA== jump%2B%2F".to_vec()
            } else {
                b"target-B jump-simulated-password".to_vec()
            };
            session.data(channel, output)?;
            session.exit_status_request(channel, 0)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }
    async fn subsystem_request(
        &mut self,
        channel: ChannelId,
        name: &str,
        session: &mut server::Session,
    ) -> Result<()> {
        ensure!(name == "sftp", "Unexpected subsystem");
        let channel = self.channels.remove(&channel).unwrap();
        session.channel_success(channel.id())?;
        self.tasks.push(tokio::spawn(async move {
            let path = if cfg!(target_os = "macos") {
                "/usr/libexec/sftp-server"
            } else {
                "/usr/lib/openssh/sftp-server"
            };
            let mut child = tokio::process::Command::new(path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            let mut stdout = child.stdout.take().unwrap();
            let (mut reader, mut writer) = tokio::io::split(channel.into_stream());
            let _ = tokio::try_join!(
                tokio::io::copy(&mut reader, &mut stdin),
                tokio::io::copy(&mut stdout, &mut writer)
            );
        }));
        Ok(())
    }
}
struct Fixture {
    _dir: tempfile::TempDir,
    vault: Vault,
    target: Uuid,
    jump: Uuid,
    a: TestServer,
    b: TestServer,
    a_port: u16,
    b_port: u16,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn start(
    seed: u8,
    forward_port: Option<u16>,
    allow_forward: bool,
) -> (u16, TestServer, tokio::task::JoinHandle<()>) {
    start_with_certificates(seed, forward_port, allow_forward, None, None).await
}

async fn start_with_certificates(
    seed: u8,
    forward_port: Option<u16>,
    allow_forward: bool,
    host_certificate: Option<ssh_key::Certificate>,
    certificate_authority: Option<ssh_key::PublicKey>,
) -> (u16, TestServer, tokio::task::JoinHandle<()>) {
    let mut server = TestServer {
        auth: Default::default(),
        active: Default::default(),
        forwards: Default::default(),
        forward_port,
        allow_forward,
        public: key(3).public_key().clone(),
        certificate_authority,
    };
    let copy = server.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![key(seed)],
        certificates: host_certificate.into_iter().collect(),
        ..Default::default()
    });
    let task = tokio::spawn(async move {
        let _ = server.run_on_socket(config, &listener).await;
    });
    (port, copy, task)
}
fn input(name: &str, host: &str, port: u16, username: &str) -> ConnectionInput {
    serde_json::from_value(serde_json::json!({"name":name,"itemType":"ssh","sshAuthType":"password","modules":[{"kind":"host","value":host},{"kind":"port","value":port.to_string()},{"kind":"username"},{"kind":"password"}],"secrets":{"password":"jump-simulated-password","namedSecrets":{"username":username}}})).unwrap()
}
impl Fixture {
    async fn new(allow_forward: bool) -> Self {
        let (b_port, b, b_task) = start(2, None, true).await;
        let (a_port, a, a_task) = start(1, Some(b_port), allow_forward).await;
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path().join("vault")).unwrap();
        let jump = vault
            .save_connection(input("A", "127.0.0.1", a_port, "jump-user"))
            .unwrap()
            .id;
        let mut target = input("B", "private.test.invalid", b_port, "target-user");
        target.ssh_jump_item_id = Some(jump);
        target.ssh_auth_type = "privateKey".into();
        target.modules.retain(|module| module.kind != "password");
        target
            .modules
            .push(serde_json::from_value(serde_json::json!({"kind":"privateKey"})).unwrap());
        target.secrets.password = None;
        target.secrets.private_key = Some(
            key(3)
                .to_openssh(ssh_key::LineEnding::LF)
                .unwrap()
                .to_string(),
        );
        let private = target.secrets.private_key.clone().unwrap();
        let target = vault
            .save_connection_with_private_key(target, None, Some((&private, "test key")))
            .unwrap()
            .id;
        vault
            .set_ssh_host_trust(
                jump,
                "127.0.0.1",
                a_port,
                Some(
                    &key(1)
                        .public_key()
                        .fingerprint(ssh_key::HashAlg::Sha256)
                        .to_string(),
                ),
            )
            .unwrap();
        vault
            .set_ssh_host_trust(
                target,
                "private.test.invalid",
                b_port,
                Some(
                    &key(2)
                        .public_key()
                        .fingerprint(ssh_key::HashAlg::Sha256)
                        .to_string(),
                ),
            )
            .unwrap();
        Self {
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
    async fn command(&self, command: &str, timeout: Option<u64>) -> Result<executor::SshResponse> {
        executor::execute_ssh(
            &self.vault,
            &self.vault.get_connection(self.target)?,
            command,
            None,
            timeout,
            &HashMap::new(),
            None,
        )
        .await
    }
    async fn closed(&self) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while self.a.active.load(Ordering::SeqCst) != 0
                || self.b.active.load(Ordering::SeqCst) != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("A and B connections must close");
    }
}

#[tokio::test]
async fn pinned_plain_key_does_not_implicitly_trust_a_host_certificate() {
    let host = key(2);
    let mut builder = ssh_key::certificate::Builder::new(
        vec![1; 32],
        host.public_key().key_data().clone(),
        0,
        u64::MAX,
    )
    .unwrap();
    builder.cert_type(ssh_key::certificate::CertType::Host).unwrap();
    builder.valid_principal("test.invalid").unwrap();
    let certificate = builder.sign(&key(1)).unwrap();
    let mut handler = SshClient {
        policy: ServerKeyPolicy::Pinned(
            host.public_key().fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        ),
        observed: Default::default(),
    };
    assert!(!client::Handler::check_server_key(&mut handler, &certificate.into()).await.unwrap());
    assert!(handler.observed.lock().unwrap().as_ref().unwrap().rejection.is_some());
}

#[tokio::test]
async fn direct_untrusted_host_stops_before_authentication() {
    let fixture = Fixture::new(true).await;
    fixture
        .vault
        .set_ssh_host_trust(fixture.jump, "127.0.0.1", fixture.a_port, None)
        .unwrap();
    let target = fixture.vault.get_connection(fixture.jump).unwrap();
    let result = executor::test_connection(&fixture.vault, &target).await;
    assert_eq!(
        fixture.a.auth.load(Ordering::SeqCst),
        0,
        "Untrusted direct host received credentials"
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("host key is not trusted"), "{error}");
    fixture.closed().await;
}

#[cfg(feature = "gui")]
#[tokio::test]
async fn imported_routes_require_explicit_trust_and_never_fall_back_to_direct() {
    use crate::commands::ssh_config_import::SshConfigImport;

    let fixture = Fixture::new(true).await;
    let folder = tempfile::tempdir().unwrap();
    let data_dir = folder.path().join("vault");
    let vault = Vault::open(data_dir.clone()).unwrap();
    let source = format!(
        "Host import-jump\n HostName 127.0.0.1\n Port {}\n User jump-user\n\
         Host import-target\n HostName private.test.invalid\n Port {}\n User target-user\n ProxyJump import-jump\n\
         Host import-missing\n HostName 127.0.0.1\n Port {}\n User jump-user\n ProxyJump not-selected\n",
        fixture.a_port, fixture.b_port, fixture.a_port
    );
    let parsed = SshConfigImport::parse(&source).unwrap();
    let (summary, _) = parsed.commit(&vault, vec![0, 1, 2]).unwrap();
    assert_eq!(summary.added, 3);
    drop(vault);
    let vault = Vault::open(data_dir).unwrap();
    let items = vault.list_connections().unwrap();
    let id = |name: &str| items.iter().find(|item| item.name == name).unwrap().id;
    let (jump, target, missing) = (id("import-jump"), id("import-target"), id("import-missing"));
    assert!(vault.ssh_host_trust().unwrap().is_empty());
    let missing = vault.get_connection(missing).unwrap();
    assert!(
        connect(&vault, &missing)
            .await
            .err()
            .expect("unresolved import must block transport")
            .to_string()
            .contains("jump reference is unresolved")
    );
    let direct = vault.get_connection(jump).unwrap();
    assert!(
        connect(&vault, &direct)
            .await
            .err()
            .expect("untrusted import must block transport")
            .to_string()
            .contains("host key is not trusted")
    );
    assert_eq!(fixture.a.auth.load(Ordering::SeqCst), 0);
    assert!(fixture.a.forwards.lock().unwrap().is_empty());

    let mut jump_input = input("import-jump", "127.0.0.1", fixture.a_port, "jump-user");
    jump_input.id = Some(jump);
    vault.save_connection(jump_input).unwrap();
    let mut target_input = input(
        "import-target",
        "private.test.invalid",
        fixture.b_port,
        "target-user",
    );
    target_input.id = Some(target);
    let saved_target = vault.get_connection(target).unwrap();
    assert_eq!(saved_target.stored.ssh_jump_item_id, Some(jump));
    target_input.ssh_jump_item_id = saved_target.stored.ssh_jump_item_id;
    target_input.secrets.password = Some("target-simulated-password".into());
    vault.save_connection(target_input).unwrap();
    let fingerprint = |seed| {
        key(seed)
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string()
    };
    vault
        .set_ssh_host_trust(jump, "127.0.0.1", fixture.a_port, Some(&fingerprint(9)))
        .unwrap();
    let direct = vault.get_connection(jump).unwrap();
    assert!(
        executor::test_connection(&vault, &direct)
            .await
            .unwrap_err()
            .to_string()
            .contains("host key mismatch")
    );
    assert_eq!(fixture.a.auth.load(Ordering::SeqCst), 0);
    vault
        .set_ssh_host_trust(jump, "127.0.0.1", fixture.a_port, Some(&fingerprint(1)))
        .unwrap();
    vault
        .set_ssh_host_trust(
            target,
            "private.test.invalid",
            fixture.b_port,
            Some(&fingerprint(9)),
        )
        .unwrap();
    let target_item = vault.get_connection(target).unwrap();
    assert!(
        executor::test_connection(&vault, &target_item)
            .await
            .unwrap_err()
            .to_string()
            .contains("host key mismatch")
    );
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
    vault
        .set_ssh_host_trust(
            target,
            "private.test.invalid",
            fixture.b_port,
            Some(&fingerprint(2)),
        )
        .unwrap();
    assert_eq!(
        executor::test_connection(&vault, &target_item)
            .await
            .unwrap(),
        "SSH connection successful"
    );
    assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
    assert_eq!(
        *fixture.a.forwards.lock().unwrap(),
        vec![("private.test.invalid".into(), u32::from(fixture.b_port)); 2]
    );
    fixture.closed().await;
}

#[tokio::test]
async fn direct_key_mismatch_stops_before_authentication() {
    let fixture = Fixture::new(true).await;
    let wrong = key(9)
        .public_key()
        .fingerprint(ssh_key::HashAlg::Sha256)
        .to_string();
    fixture
        .vault
        .set_ssh_host_trust(fixture.jump, "127.0.0.1", fixture.a_port, Some(&wrong))
        .unwrap();
    let target = fixture.vault.get_connection(fixture.jump).unwrap();
    let result = executor::test_connection(&fixture.vault, &target).await;
    assert_eq!(
        fixture.a.auth.load(Ordering::SeqCst),
        0,
        "Direct host with a mismatched key received credentials"
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("host key mismatch"), "{error}");
    fixture.closed().await;
}

#[tokio::test]
async fn direct_trust_is_bound_to_the_requested_endpoint() {
    let fixture = Fixture::new(true).await;
    let mut target = fixture.vault.get_connection(fixture.jump).unwrap();
    target.stored.host = "localhost".into();
    let result = executor::test_connection(&fixture.vault, &target).await;
    assert_eq!(
        fixture.a.auth.load(Ordering::SeqCst),
        0,
        "Trust for another hostname allowed direct authentication"
    );
    let error = result.unwrap_err().to_string();
    assert!(error.contains("host key is not trusted"), "{error}");
    fixture.closed().await;
}

#[tokio::test]
async fn direct_trusted_host_executes_connection_check_without_forwarding() {
    let fixture = Fixture::new(true).await;
    let target = fixture.vault.get_connection(fixture.jump).unwrap();
    assert_eq!(
        executor::test_connection(&fixture.vault, &target).await.unwrap(),
        "SSH connection successful"
    );
    assert!(fixture.a.auth.load(Ordering::SeqCst) > 0);
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
    assert!(fixture.a.forwards.lock().unwrap().is_empty());
    fixture.closed().await;
}

#[tokio::test]
async fn direct_probe_observes_without_authentication_or_implicit_trust() {
    let fixture = Fixture::new(true).await;
    fixture
        .vault
        .set_ssh_host_trust(fixture.jump, "127.0.0.1", fixture.a_port, None)
        .unwrap();
    let observed = probe(&fixture.vault, fixture.jump).await.unwrap();
    assert_eq!(
        observed.fingerprint,
        key(1)
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string()
    );
    assert_eq!(fixture.a.auth.load(Ordering::SeqCst), 0);
    assert!(
        fixture.vault.ssh_host_trust().unwrap().iter()
            .all(|entry| entry.item_id != fixture.jump)
    );
    fixture.closed().await;
}

#[tokio::test]
async fn jump_connection_check_accepts_windows_line_endings_and_uses_target_route() {
    let fixture = Fixture::new(true).await;
    let target = fixture.vault.get_connection(fixture.target).unwrap();
    assert_eq!(
        executor::test_connection(&fixture.vault, &target).await.unwrap(),
        "SSH connection successful"
    );
    assert!(fixture.a.auth.load(Ordering::SeqCst) > 0);
    assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
    fixture.closed().await;
    assert_eq!(
        *fixture.a.forwards.lock().unwrap(),
        vec![("private.test.invalid".into(), u32::from(fixture.b_port))]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn jump_command_and_real_sftp_use_separate_credentials_and_close_both_connections() {
    let sftp_server = if cfg!(target_os = "macos") {
        "/usr/libexec/sftp-server"
    } else {
        "/usr/lib/openssh/sftp-server"
    };
    assert!(
        std::path::Path::new(sftp_server).exists(),
        "Real SFTP fixture requires the system openssh-sftp-server executable"
    );
    let fixture = Fixture::new(true).await;
    let result = fixture.command("hostname", Some(10)).await.unwrap();
    assert_eq!(result.stdout, "target-B [REDACTED]");
    assert!(fixture.a.auth.load(Ordering::SeqCst) > 0);
    assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
    fixture.closed().await;
    let dir = tempfile::tempdir().unwrap();
    let local = dir.path().join("source");
    let remote = dir.path().join("remote");
    let downloaded = dir.path().join("downloaded");
    let content = vec![0xA5; 131_073];
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
    assert_eq!(tokio::fs::read(downloaded).await.unwrap(), content);
    fixture.closed().await;
    let missing = dir.path().join("missing-file");
    assert!(
        executor::ssh_download(
            &fixture.vault,
            &target,
            missing.to_str().unwrap(),
            dir.path().join("unused").to_str().unwrap(),
            true,
            Some(10)
        )
        .await
        .is_err()
    );
    fixture.closed().await;
    let forwards = fixture.a.forwards.lock().unwrap();
    assert_eq!(forwards.len(), 4);
    assert!(
        forwards
            .iter()
            .all(|entry| entry == &("private.test.invalid".into(), u32::from(fixture.b_port)))
    );
}

#[tokio::test]
async fn jump_key_mismatches_stop_before_authentication() {
    for wrong_a in [true, false] {
        let fixture = Fixture::new(true).await;
        let wrong = key(9)
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string();
        if wrong_a {
            fixture
                .vault
                .set_ssh_host_trust(fixture.jump, "127.0.0.1", fixture.a_port, Some(&wrong))
                .unwrap();
        } else {
            fixture
                .vault
                .set_ssh_host_trust(
                    fixture.target,
                    "private.test.invalid",
                    fixture.b_port,
                    Some(&wrong),
                )
                .unwrap();
        }
        let error = fixture
            .command("hostname", Some(10))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("host key mismatch"), "{error}");
        assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
        if wrong_a {
            assert_eq!(fixture.a.auth.load(Ordering::SeqCst), 0);
            assert!(fixture.a.forwards.lock().unwrap().is_empty());
        }
        fixture.closed().await;
    }
}

#[tokio::test]
async fn probe_never_authenticates_target_and_forward_denial_has_no_fallback() {
    let fixture = Fixture::new(true).await;
    let observed = probe(&fixture.vault, fixture.target).await.unwrap();
    assert_eq!(
        observed.fingerprint,
        key(2)
            .public_key()
            .fingerprint(ssh_key::HashAlg::Sha256)
            .to_string()
    );
    assert_eq!(fixture.b.auth.load(Ordering::SeqCst), 0);
    fixture.closed().await;
    let auth_before = fixture.a.auth.load(Ordering::SeqCst);
    let a_probe = probe(&fixture.vault, fixture.jump).await.unwrap();
    assert_eq!(a_probe.port, fixture.a_port);
    assert_eq!(fixture.a.auth.load(Ordering::SeqCst), auth_before);
    fixture.closed().await;
    let denied = Fixture::new(false).await;
    assert!(
        denied
            .command("hostname", Some(10))
            .await
            .unwrap_err()
            .to_string()
            .contains("forwarding")
    );
    assert_eq!(denied.b.auth.load(Ordering::SeqCst), 0);
    denied.closed().await;
}

#[tokio::test]
async fn timeout_and_cancellation_close_both_ssh_sessions() {
    let fixture = Fixture::new(true).await;
    assert!(fixture.command("hold", Some(1)).await.is_err());
    fixture.closed().await;
    {
        let pending = fixture.command("hold", None);
        tokio::pin!(pending);
        tokio::select! { result=&mut pending=>panic!("hold unexpectedly returned {result:?}"), _=tokio::time::sleep(Duration::from_millis(300))=>{} }
    }
    fixture.closed().await;
}

#[tokio::test]
async fn all_password_and_private_key_pairs_authenticate_independently() {
    for a_mode in ["password", "privateKey"] {
        for b_mode in ["password", "privateKey"] {
            let fixture = Fixture::new(true).await;
            for (id, mode, seed, password) in [
                (fixture.jump, a_mode, 4, "jump-simulated-password"),
                (fixture.target, b_mode, 3, "target-simulated-password"),
            ] {
                let current = fixture.vault.get_connection(id).unwrap();
                let mut input: ConnectionInput = serde_json::from_value(
                    serde_json::to_value(current.stored.portable()).unwrap(),
                )
                .unwrap();
                input.secrets = current.secrets.clone();
                input.ssh_auth_type = mode.into();
                let kind = if mode == "privateKey" {
                    "privateKey"
                } else {
                    "password"
                };
                if !input.modules.iter().any(|module| module.kind == kind) {
                    input
                        .modules
                        .push(serde_json::from_value(serde_json::json!({"kind":kind})).unwrap());
                }
                let private = key(seed).to_openssh(ssh_key::LineEnding::LF).unwrap();
                if mode == "password" {
                    input.secrets.password = Some(password.into());
                }
                fixture
                    .vault
                    .save_connection_with_private_key(
                        input,
                        None,
                        if mode == "privateKey" {
                            Some((private.as_str(), "test key"))
                        } else {
                            None
                        },
                    )
                    .unwrap();
            }
            assert_eq!(
                fixture
                    .command("hostname", Some(10))
                    .await
                    .unwrap()
                    .exit_code,
                Some(0)
            );
            assert!(fixture.a.auth.load(Ordering::SeqCst) > 0);
            assert!(fixture.b.auth.load(Ordering::SeqCst) > 0);
            fixture.closed().await;
        }
    }
}

#[tokio::test]
async fn cancelling_pending_handshake_closes_the_tcp_socket() {
    use tokio::io::AsyncReadExt;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut bytes))
            .await
            .expect("cancelled handshake must close TCP")
            .unwrap();
        assert!(bytes.starts_with(b"SSH-"));
    });
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let item = vault
        .save_connection(input("silent", "127.0.0.1", port, "jump-user"))
        .unwrap();
    {
        let pending = probe(&vault, item.id);
        tokio::pin!(pending);
        tokio::select! {result=&mut pending=>panic!("silent server unexpectedly returned {}",result.is_ok()), _=tokio::time::sleep(Duration::from_millis(100))=>{} }
    }
    peer.await.unwrap();
}

#[tokio::test]
async fn jump_and_target_secrets_are_redacted_together_longest_first() {
    let fixture = Fixture::new(true).await;
    for (id, secrets) in [
        (fixture.jump, vec!["jump", "jump+/"]),
        (fixture.target, vec!["jumpsecret"]),
    ] {
        let current = fixture.vault.get_connection(id).unwrap();
        let mut input: ConnectionInput =
            serde_json::from_value(serde_json::to_value(current.stored.portable()).unwrap())
                .unwrap();
        input.secrets = current.secrets.clone();
        for (index, value) in secrets.iter().enumerate() {
            input
                .secrets
                .named_secrets
                .insert(format!("redaction{index}"), value.to_string());
        }
        fixture.vault.save_connection(input).unwrap();
    }
    let response = fixture.command("redaction", Some(10)).await.unwrap();
    assert_eq!(
        response.stdout,
        "[REDACTED] [REDACTED] [REDACTED] [REDACTED]"
    );
    let connection = fixture.vault.get_connection(fixture.target).unwrap();
    let route = connect(&fixture.vault, &connection).await.unwrap();
    assert_eq!(
        route.redact("jumpsecret jump anVtcA== jump%2B%2F".into()),
        "[REDACTED] [REDACTED] [REDACTED] [REDACTED]"
    );
    drop(route);
    fixture.closed().await;
    #[cfg(unix)]
    {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("jumpsecret jump anVtcA== jump%2B%2F");
        std::fs::create_dir(&source).unwrap();
        std::os::unix::fs::symlink(&source, source.join("loop")).unwrap();
        let target = fixture.vault.get_connection(fixture.target).unwrap();
        let error = executor::ssh_upload(
            &fixture.vault,
            &target,
            source.to_str().unwrap(),
            dir.path().join("remote").to_str().unwrap(),
            true,
            Some(10),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(!error.contains("jumpsecret"), "{error}");
        assert!(!error.contains("anVtcA=="), "{error}");
        assert!(!error.contains("jump%2B%2F"), "{error}");
        assert!(
            error.contains("[REDACTED] [REDACTED] [REDACTED] [REDACTED]"),
            "{error}"
        );
        fixture.closed().await;
    }
}

#[path = "ssh_certificate_tests.rs"]
mod certificate_tests;
