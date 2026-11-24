use crate::{
    model::SshHostTrust,
    vault::{DecryptedConnection, Vault, ssh_endpoint_host},
};
use anyhow::{Context, Result, ensure};
use russh::{
    client,
    keys::{PrivateKeyWithHashAlg, decode_secret_key, ssh_key},
};
use serde::Serialize;
use std::{
    future::Future,
    io,
    ops::{Deref, DerefMut},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context as TaskContext, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio_util::sync::{CancellationToken, WaitForCancellationFutureOwned};
use uuid::Uuid;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

struct CancellableStream<S> {
    stream: S,
    cancelled: Pin<Box<WaitForCancellationFutureOwned>>,
}
impl<S> CancellableStream<S> {
    fn new(stream: S, token: &CancellationToken) -> Self {
        Self {
            stream,
            cancelled: Box::pin(token.clone().cancelled_owned()),
        }
    }
    fn check(&mut self, cx: &mut TaskContext<'_>) -> io::Result<()> {
        if self.cancelled.as_mut().poll(cx).is_ready() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "SSH connection cancelled",
            ));
        }
        Ok(())
    }
}
impl<S: AsyncRead + Unpin> AsyncRead for CancellableStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for CancellableStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        if let Err(error) = self.check(cx) {
            return Poll::Ready(Err(error));
        }
        Pin::new(&mut self.stream).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(cx)
    }
}

pub(crate) struct SshSession {
    handle: client::Handle<SshClient>,
    _cancel: CancelOnDrop,
}
impl Deref for SshSession {
    type Target = client::Handle<SshClient>;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl DerefMut for SshSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.handle
    }
}

pub(crate) struct SshConnection {
    target: SshSession,
    _jump: Option<SshSession>,
    secrets: zeroize::Zeroizing<Vec<String>>,
}
impl Deref for SshConnection {
    type Target = client::Handle<SshClient>;
    fn deref(&self) -> &Self::Target {
        &self.target.handle
    }
}
impl SshConnection {
    pub(crate) fn redact(&self, value: String) -> String {
        crate::policy::redact_values(value, &self.secrets)
    }
}

fn route_secrets(
    target: &DecryptedConnection,
    jump: Option<&DecryptedConnection>,
) -> zeroize::Zeroizing<Vec<String>> {
    let mut values = target.secrets.non_empty_values();
    if let Some(jump) = jump {
        values.extend(jump.secrets.non_empty_values());
    }
    if target.stored.http_auth_type == "basic" {
        if let (Some(user), Some(password)) = (
            target.secrets.get("username"),
            target.secrets.get("password"),
        ) {
            values.push(format!("{user}:{password}"));
        }
    }
    zeroize::Zeroizing::new(values)
}

#[derive(Clone)]
enum ServerKeyPolicy {
    Probe,
    Pinned(String),
}

pub(crate) struct SshClient {
    policy: ServerKeyPolicy,
    observed: Arc<Mutex<Option<String>>>,
}
impl client::Handler for SshClient {
    type Error = russh::Error;
    async fn check_server_key(&mut self, key: &ssh_key::PublicKey) -> Result<bool, Self::Error> {
        let actual = key.fingerprint(ssh_key::HashAlg::Sha256).to_string();
        *self.observed.lock().unwrap() = Some(actual.clone());
        Ok(match &self.policy {
            ServerKeyPolicy::Probe => false,
            ServerKeyPolicy::Pinned(expected) => expected == &actual,
        })
    }
}

fn expected_key(trust: &[SshHostTrust], connection: &DecryptedConnection) -> Result<String> {
    let host = ssh_endpoint_host(&connection.stored.host)?;
    trust.iter().find(|entry| entry.item_id == connection.stored.id && entry.host == host && entry.port == connection.stored.port)
        .map(|entry|entry.fingerprint.clone()).context("SSH host key is not trusted; verify the fingerprint using the local client or ssh-trust CLI")
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    policy: ServerKeyPolicy,
    observed: Arc<Mutex<Option<String>>>,
) -> Result<SshSession> {
    let token = CancellationToken::new();
    let guard = CancelOnDrop(token.clone());
    let stream = CancellableStream::new(stream, &token);
    let handler = SshClient {
        policy: policy.clone(),
        observed: observed.clone(),
    };
    let config = Arc::new(client::Config::default());
    let handle = client::connect_stream(config, stream, handler)
        .await
        .map_err(|error| {
            if let (ServerKeyPolicy::Pinned(expected), Some(actual)) =
                (policy, observed.lock().unwrap().as_ref())
            {
                if expected != *actual {
                    return anyhow::anyhow!(
                        "SSH host key mismatch; expected {expected}, received {actual}"
                    );
                }
            }
            anyhow::anyhow!("SSH handshake failed: {error}")
        })?;
    Ok(SshSession {
        handle,
        _cancel: guard,
    })
}

async fn direct(connection: &DecryptedConnection, expected: String) -> Result<SshSession> {
    let stream =
        tokio::net::TcpStream::connect((connection.stored.host.as_str(), connection.stored.port))
            .await
            .context("SSH TCP connection failed")?;
    let mut session = handshake(
        stream,
        ServerKeyPolicy::Pinned(expected),
        Default::default(),
    )
    .await?;
    authenticate(&mut session, connection).await?;
    Ok(session)
}

async fn authenticate(session: &mut SshSession, connection: &DecryptedConnection) -> Result<()> {
    let username = connection
        .secrets
        .get("username")
        .context("No SSH username in vault")?;
    let result = if connection.stored.ssh_auth_type == "privateKey" {
        let key = decode_secret_key(
            connection
                .secrets
                .private_key
                .as_deref()
                .context("No SSH private key in vault")?,
            connection.secrets.passphrase.as_deref(),
        )
        .context("Unable to parse SSH private key or incorrect passphrase")?;
        let hash = session.best_supported_rsa_hash().await?.flatten();
        session
            .authenticate_publickey(username, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await?
    } else {
        session
            .authenticate_password(
                username,
                connection
                    .secrets
                    .password
                    .clone()
                    .context("No SSH password in vault")?,
            )
            .await?
    };
    ensure!(result.success(), "SSH authentication failed");
    Ok(())
}

pub(crate) async fn connect(
    vault: &Vault,
    connection: &DecryptedConnection,
) -> Result<SshConnection> {
    let jump = vault.ssh_jump_connection(connection)?;
    let operation = async {
        if let Some(jump) = &jump {
            let trust = vault.ssh_host_trust()?;
            let a_key = expected_key(&trust, jump).context("Jump host A")?;
            let b_key = expected_key(&trust, connection).context("Target host B")?;
            let a = direct(jump, a_key).await.context("Jump host A")?;
            let channel = a
                .channel_open_direct_tcpip(
                    connection.stored.host.clone(),
                    u32::from(connection.stored.port),
                    "127.0.0.1",
                    0,
                )
                .await
                .context("Jump host A forwarding to target B failed")?;
            let mut b = handshake(
                channel.into_stream(),
                ServerKeyPolicy::Pinned(b_key),
                Default::default(),
            )
            .await
            .context("Target host B")?;
            authenticate(&mut b, connection)
                .await
                .context("Target host B")?;
            Ok(SshConnection {
                target: b,
                _jump: Some(a),
                secrets: route_secrets(connection, Some(jump)),
            })
        } else {
            let expected = expected_key(&vault.ssh_host_trust()?, connection)?;
            Ok(SshConnection {
                target: direct(connection, expected).await?,
                _jump: None,
                secrets: route_secrets(connection, None),
            })
        }
    };
    let result = tokio::time::timeout(CONNECT_TIMEOUT, operation)
        .await
        .context("SSH connection timed out")?;
    result.map_err(|error: anyhow::Error| {
        anyhow::anyhow!(crate::policy::redact_values(
            format!("{error:#}"),
            &route_secrets(connection, jump.as_ref())
        ))
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshHostProbe {
    pub host: String,
    pub port: u16,
    pub fingerprint: String,
}

pub async fn probe(vault: &Vault, id: Uuid) -> Result<SshHostProbe> {
    let target = vault.get_connection(id)?;
    ensure!(target.stored.enabled, "SSH project disabled");
    let jump = vault.ssh_jump_connection(&target)?;
    let result = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let a;
        let observed = Arc::new(Mutex::new(None));
        if let Some(jump) = &jump {
            a = direct(jump, expected_key(&vault.ssh_host_trust()?, jump)?)
                .await
                .context("Jump host A")?;
            let channel = a
                .channel_open_direct_tcpip(
                    target.stored.host.clone(),
                    u32::from(target.stored.port),
                    "127.0.0.1",
                    0,
                )
                .await
                .context("Jump forwarding failed")?;
            let _ = handshake(
                channel.into_stream(),
                ServerKeyPolicy::Probe,
                observed.clone(),
            )
            .await;
        } else {
            let stream =
                tokio::net::TcpStream::connect((target.stored.host.as_str(), target.stored.port))
                    .await
                    .context("SSH TCP connection failed")?;
            let _ = handshake(stream, ServerKeyPolicy::Probe, observed.clone()).await;
        }
        let fingerprint = observed
            .lock()
            .unwrap()
            .clone()
            .context("SSH host did not provide a public key")?;
        Ok(SshHostProbe {
            host: ssh_endpoint_host(&target.stored.host)?,
            port: target.stored.port,
            fingerprint,
        })
    })
    .await
    .context("SSH host key probe timed out")?;
    result.map_err(|error: anyhow::Error| {
        anyhow::anyhow!(crate::policy::redact_values(
            format!("{error:#}"),
            &route_secrets(&target, jump.as_ref())
        ))
    })
}

#[cfg(test)]
#[path = "ssh_transport_tests.rs"]
mod tests;
