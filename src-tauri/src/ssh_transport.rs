use crate::{
    model::{SshHostTrust, SshHostTrustKind},
    vault::{DecryptedConnection, Vault, ssh_endpoint_host},
};
use anyhow::{Context, Result, ensure};
use russh::{
    client,
    keys::{PrivateKeyWithHashAlg, PublicKeyOrCertificate, decode_secret_key, ssh_key},
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
    CertificateAuthority { fingerprint: String, hostname: String },
}

#[derive(Clone)]
struct ObservedServerKey {
    fingerprint: String,
    ca_fingerprint: Option<String>,
    rejection: Option<String>,
}

pub(crate) struct SshClient {
    policy: ServerKeyPolicy,
    observed: Arc<Mutex<Option<ObservedServerKey>>>,
}
impl client::Handler for SshClient {
    type Error = russh::Error;
    async fn check_server_key(&mut self, presented: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fingerprint = presented.public_key().fingerprint(ssh_key::HashAlg::Sha256).to_string();
        let ca_fingerprint = presented.certificate().map(|cert| cert.signature_key().fingerprint(ssh_key::HashAlg::Sha256).to_string());
        let verdict = match (&self.policy, presented) {
            (ServerKeyPolicy::Probe, _) => Ok(false),
            (ServerKeyPolicy::Pinned(expected), PublicKeyOrCertificate::PublicKey { .. }) => Ok(expected == &fingerprint),
            (ServerKeyPolicy::Pinned(_), PublicKeyOrCertificate::Certificate(_)) => Err(anyhow::anyhow!("Expected a pinned SSH host key, received a certificate")),
            (ServerKeyPolicy::CertificateAuthority { fingerprint, hostname }, PublicKeyOrCertificate::Certificate(cert)) => {
                crate::certificates::verify_ssh_host_certificate(cert, hostname, fingerprint, chrono::Utc::now().timestamp()).map(|_| true)
            }
            (ServerKeyPolicy::CertificateAuthority { .. }, PublicKeyOrCertificate::PublicKey { .. }) => Err(anyhow::anyhow!("Expected an SSH host certificate")),
        };
        let accepted = verdict.as_ref().copied().unwrap_or(false);
        *self.observed.lock().unwrap() = Some(ObservedServerKey {
            fingerprint,
            ca_fingerprint,
            rejection: verdict.err().map(|error| error.to_string()),
        });
        Ok(accepted)
    }
}

fn expected_policy(trust: &[SshHostTrust], connection: &DecryptedConnection) -> Result<ServerKeyPolicy> {
    let host = ssh_endpoint_host(&connection.stored.host)?;
    let entry = trust.iter().find(|entry| entry.item_id == connection.stored.id && entry.host == host && entry.port == connection.stored.port)
        .context("SSH host key is not trusted; verify the fingerprint using the local client or ssh-trust CLI")?;
    Ok(match entry.kind {
        SshHostTrustKind::HostKey => ServerKeyPolicy::Pinned(entry.fingerprint.clone()),
        SshHostTrustKind::CertificateAuthority => ServerKeyPolicy::CertificateAuthority { fingerprint: entry.fingerprint.clone(), hostname: host },
    })
}

async fn handshake<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(
    stream: S,
    policy: ServerKeyPolicy,
    observed: Arc<Mutex<Option<ObservedServerKey>>>,
) -> Result<SshSession> {
    let token = CancellationToken::new();
    let guard = CancelOnDrop(token.clone());
    let stream = CancellableStream::new(stream, &token);
    let handler = SshClient { policy: policy.clone(), observed: observed.clone() };
    let mut config = client::Config::default();
    if matches!(policy, ServerKeyPolicy::Probe | ServerKeyPolicy::CertificateAuthority { .. }) {
        config.preferred.host_key_certificates = config.preferred.key.clone();
    }
    let handle = client::connect_stream(Arc::new(config), stream, handler)
        .await
        .map_err(|error| {
            if let Some(actual) = observed.lock().unwrap().as_ref() {
                if let Some(rejection) = &actual.rejection {
                    return anyhow::anyhow!("SSH host verification failed: {rejection}");
                }
                if let ServerKeyPolicy::Pinned(expected) = &policy {
                    if expected != &actual.fingerprint {
                        return anyhow::anyhow!("SSH host key mismatch; expected {expected}, received {}", actual.fingerprint);
                    }
                }
            }
            anyhow::anyhow!("SSH handshake failed: {error}")
        })?;
    Ok(SshSession { handle, _cancel: guard })
}

async fn direct(connection: &DecryptedConnection, policy: ServerKeyPolicy) -> Result<SshSession> {
    let stream = tokio::net::TcpStream::connect((connection.stored.host.as_str(), connection.stored.port))
        .await.context("SSH TCP connection failed")?;
    let mut session = handshake(stream, policy, Default::default()).await?;
    authenticate(&mut session, connection).await?;
    Ok(session)
}

async fn authenticate(session: &mut SshSession, connection: &DecryptedConnection) -> Result<()> {
    let username = connection
        .secrets
        .get("username")
        .context("No SSH username in vault")?;
    let result = if matches!(connection.stored.ssh_auth_type.as_str(), "privateKey" | "certificate") {
        let key = decode_secret_key(
            connection
                .secrets
                .private_key
                .as_deref()
                .context("No SSH private key in vault")?,
            connection.secrets.passphrase.as_deref(),
        )
        .context("Unable to parse SSH private key or incorrect passphrase")?;
        if connection.stored.ssh_auth_type == "certificate" {
            let mut modules = connection.stored.modules.iter().filter(|module| module.kind == "sshCertificate" && !module.value.trim().is_empty());
            let module = modules.next().context("No SSH user certificate in vault")?;
            ensure!(modules.next().is_none(), "Multiple configured SSH user certificates");
            ensure!(module.value.len() <= 1024 * 1024, "SSH user certificate is too large");
            let certificate = ssh_key::Certificate::from_openssh(module.value.trim()).context("Invalid SSH user certificate")?;
            ensure!(certificate.cert_type() == ssh_key::certificate::CertType::User, "SSH authentication requires a user certificate");
            ensure!(key.public_key().key_data() == certificate.public_key(), "SSH certificate does not match the private key");
            let now = u64::try_from(chrono::Utc::now().timestamp()).context("Invalid local clock")?;
            ensure!(certificate.valid_after() <= now && now < certificate.valid_before(), "SSH user certificate is outside its validity period");
            ensure!(certificate.critical_options().is_empty(), "SSH user certificate critical options are not supported");
            // The server decides which CA and principals authorize this account.
            certificate.verify_signature().context("Invalid SSH user certificate signature")?;
            session.authenticate_openssh_cert(username, Arc::new(key), certificate).await?
        } else {
            let hash = session.best_supported_rsa_hash().await?.flatten();
            session.authenticate_publickey(username, PrivateKeyWithHashAlg::new(Arc::new(key), hash)).await?
        }
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
            let a_key = expected_policy(&trust, jump).context("Jump host A")?;
            let b_key = expected_policy(&trust, connection).context("Target host B")?;
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
                b_key,
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
            let expected = expected_policy(&vault.ssh_host_trust()?, connection)?;
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
    /// SHA256 of the underlying host public key, including certificate presentations.
    pub fingerprint: String,
    pub kind: SshHostTrustKind,
    /// Observed signer only; selecting CA trust requires explicit owner approval.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ca_fingerprint: Option<String>,
}

pub async fn probe(vault: &Vault, id: Uuid) -> Result<SshHostProbe> {
    let target = vault.get_connection(id)?;
    ensure!(target.stored.enabled, "SSH project disabled");
    let jump = vault.ssh_jump_connection(&target)?;
    let result = tokio::time::timeout(CONNECT_TIMEOUT, async {
        let a;
        let observed = Arc::new(Mutex::new(None));
        if let Some(jump) = &jump {
            a = direct(jump, expected_policy(&vault.ssh_host_trust()?, jump)?)
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
        let observation = observed
            .lock()
            .unwrap()
            .clone()
            .context("SSH host did not provide a public key")?;
        Ok(SshHostProbe {
            host: ssh_endpoint_host(&target.stored.host)?,
            port: target.stored.port,
            fingerprint: observation.fingerprint,
            kind: if observation.ca_fingerprint.is_some() { SshHostTrustKind::CertificateAuthority } else { SshHostTrustKind::HostKey },
            ca_fingerprint: observation.ca_fingerprint,
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
