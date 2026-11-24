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
        public: &ssh_key::PublicKey,
    ) -> Result<server::Auth> {
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
    let mut server = TestServer {
        auth: Default::default(),
        active: Default::default(),
        forwards: Default::default(),
        forward_port,
        allow_forward,
        public: key(3).public_key().clone(),
    };
    let copy = server.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(server::Config {
        keys: vec![key(seed)],
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
