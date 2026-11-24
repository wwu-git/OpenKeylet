use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[path = "certificates/ssh.rs"]
mod ssh;
#[cfg(test)]
#[path = "certificates/tests.rs"]
mod tests;
#[path = "certificates/tls.rs"]
mod tls;

pub const MAX_INPUT_BYTES: usize = 1024 * 1024;
pub const MAX_VALIDITY_SECONDS: u64 = 366 * 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CertificateKind {
    SshUser,
    SshHost,
    TlsServer,
    TlsClient,
}

impl CertificateKind {
    pub fn is_ssh(self) -> bool {
        matches!(self, Self::SshUser | Self::SshHost)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CertificateRequest {
    SshUser {
        public_key: String,
        principals: Vec<String>,
        key_id: String,
        #[serde(default)]
        extensions: Vec<String>,
    },
    SshHost {
        public_key: String,
        principals: Vec<String>,
        key_id: String,
    },
    TlsServer {
        csr: String,
        names: Vec<String>,
    },
    TlsClient {
        csr: String,
        names: Vec<String>,
    },
}

impl CertificateRequest {
    pub fn kind(&self) -> CertificateKind {
        match self {
            Self::SshUser { .. } => CertificateKind::SshUser,
            Self::SshHost { .. } => CertificateKind::SshHost,
            Self::TlsServer { .. } => CertificateKind::TlsServer,
            Self::TlsClient { .. } => CertificateKind::TlsClient,
        }
    }
}

pub struct IssuerMaterial<'a> {
    pub private_key: &'a str,
    pub passphrase: Option<&'a str>,
    pub ca_certificate: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssuerIdentity {
    pub key_fingerprint: String,
    pub certificate_fingerprint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IssuedCertificate {
    pub certificate: String,
    pub chain: Vec<String>,
    pub serial: String,
    pub issuer_fingerprint: String,
    pub public_key_fingerprint: String,
    pub valid_after: i64,
    pub valid_before: i64,
    pub kind: CertificateKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CertificateSummary {
    pub kind: Option<CertificateKind>,
    pub is_ca: bool,
    pub identities: Vec<String>,
    pub serial: String,
    pub issuer_fingerprint: String,
    pub public_key_fingerprint: String,
    pub valid_after: i64,
    pub valid_before: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KeyMatchReport {
    pub algorithm: String,
    pub fingerprint: String,
    pub matches: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerifyRequest {
    pub certificate: String,
    pub authorities: Vec<String>,
    #[serde(default)]
    pub intermediates: Vec<String>,
    pub identity: String,
    pub kind: CertificateKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VerificationReport {
    pub verified: bool,
    pub signature_valid: bool,
    pub issuer_trusted: bool,
    pub time_valid: bool,
    pub identity_matches: bool,
    pub purpose_valid: bool,
    pub revocation_checked: bool,
    pub reasons: Vec<String>,
    pub summary: Option<CertificateSummary>,
}

impl VerificationReport {
    fn new(summary: CertificateSummary) -> Self {
        Self {
            verified: false,
            signature_valid: false,
            issuer_trusted: false,
            time_valid: false,
            identity_matches: false,
            purpose_valid: false,
            revocation_checked: false,
            reasons: Vec::new(),
            summary: Some(summary),
        }
    }
    fn finish(mut self) -> Self {
        self.verified = self.signature_valid
            && self.issuer_trusted
            && self.time_valid
            && self.identity_matches
            && self.purpose_valid;
        if !self.signature_valid {
            self.reasons
                .push("Certificate signature could not be verified".into());
        }
        if !self.issuer_trusted {
            self.reasons
                .push("Certificate issuer is not trusted".into());
        }
        if !self.time_valid {
            self.reasons
                .push("Certificate is not valid at the current time".into());
        }
        if !self.identity_matches {
            self.reasons
                .push("Certificate identity does not match".into());
        }
        if !self.purpose_valid {
            self.reasons
                .push("Certificate purpose or constraints do not match".into());
        }
        self.reasons.push("Revocation was not checked".into());
        self
    }
}

pub fn issuer_identity(
    material: &IssuerMaterial<'_>,
    kind: CertificateKind,
    now: i64,
) -> Result<IssuerIdentity> {
    bounded(material.private_key)?;
    if let Some(p) = material.passphrase {
        ensure!(p.len() <= MAX_INPUT_BYTES, "Oversized passphrase");
    }
    if kind.is_ssh() {
        ssh::issuer_identity(material)
    } else {
        tls::issuer_identity(material, now)
    }
}

pub fn check_key(
    private: &str,
    passphrase: Option<&str>,
    public_or_certificate: &str,
) -> Result<KeyMatchReport> {
    bounded(private)?;
    bounded(public_or_certificate)?;
    if let Some(p) = passphrase {
        ensure!(p.len() <= MAX_INPUT_BYTES, "Oversized passphrase");
    }
    if public_or_certificate
        .trim_start()
        .starts_with("-----BEGIN ")
    {
        tls::check_key(private, passphrase, public_or_certificate)
    } else {
        ssh::check_key(private, passphrase, public_or_certificate)
    }
}

pub fn inspect_certificate(input: &str) -> Result<CertificateSummary> {
    bounded(input)?;
    if input
        .trim_start()
        .starts_with("-----BEGIN CERTIFICATE-----")
    {
        tls::inspect(input)
    } else {
        ssh::inspect(input)
    }
}

pub fn verify_certificate(input: &VerifyRequest, now: i64) -> Result<VerificationReport> {
    bounded(&input.certificate)?;
    ensure!(now >= 0, "Invalid verification time");
    ensure!(
        !input.authorities.is_empty() && input.authorities.len() <= 100,
        "Supply between 1 and 100 explicit trusted authorities"
    );
    ensure!(
        input.intermediates.len() <= 20,
        "Too many intermediate certificates"
    );
    for authority in input.authorities.iter().chain(input.intermediates.iter()) {
        bounded(authority)?;
    }
    identities(std::slice::from_ref(&input.identity))?;
    if input.kind.is_ssh() {
        ssh::verify(input, now)
    } else {
        tls::verify(input, now)
    }
}

pub fn issue_certificate(
    material: &IssuerMaterial<'_>,
    request: &CertificateRequest,
    ttl_seconds: u64,
    serial: &[u8],
    now: i64,
) -> Result<IssuedCertificate> {
    bounded(material.private_key)?;
    if let Some(p) = material.passphrase {
        ensure!(p.len() <= MAX_INPUT_BYTES, "Oversized passphrase");
    }
    ensure!(
        now >= 0 && ttl_seconds > 0 && ttl_seconds <= MAX_VALIDITY_SECONDS,
        "Invalid certificate validity"
    );
    let end = now
        .checked_add(i64::try_from(ttl_seconds)?)
        .ok_or_else(|| anyhow::anyhow!("Invalid certificate validity"))?;
    ensure!(
        !serial.is_empty() && serial.iter().any(|b| *b != 0),
        "Certificate serial must be nonzero"
    );
    if request.kind().is_ssh() {
        ssh::issue(material, request, serial, now, end)
    } else {
        tls::issue(material, request, serial, now, end)
    }
}

pub fn verify_ssh_host_certificate(
    cert: &russh::keys::ssh_key::Certificate,
    hostname: &str,
    trusted_ca_fingerprint: &str,
    now: i64,
) -> Result<()> {
    ssh::verify_host(cert, hostname, trusted_ca_fingerprint, now)
}

fn bounded(input: &str) -> Result<()> {
    ensure!(
        !input.trim().is_empty() && input.len() <= MAX_INPUT_BYTES,
        "Empty or oversized certificate material"
    );
    Ok(())
}

fn identities(names: &[String]) -> Result<()> {
    ensure!(
        !names.is_empty() && names.len() <= 100,
        "Supply between 1 and 100 identities"
    );
    for name in names {
        ensure!(
            !name.is_empty()
                && name.len() <= 253
                && name.trim() == name
                && !name.chars().any(char::is_control),
            "Invalid certificate identity"
        );
    }
    ensure!(
        names.iter().collect::<std::collections::HashSet<_>>().len() == names.len(),
        "Duplicate certificate identity"
    );
    Ok(())
}

// TLS fingerprints hash SPKI DER; SSH fingerprints use the OpenSSH public-key blob.
fn fingerprint(bytes: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(bytes)))
}
