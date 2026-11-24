use super::*;
use anyhow::{Context, bail};
use russh::keys::{
    decode_secret_key,
    ssh_key::{
        self, Certificate, HashAlg, PublicKey,
        certificate::{Builder, CertType},
        public::KeyData,
    },
};

const EXTENSIONS: &[&str] = &[
    "permit-X11-forwarding",
    "permit-agent-forwarding",
    "permit-port-forwarding",
    "permit-pty",
    "permit-user-rc",
];

fn private(material: &IssuerMaterial<'_>) -> Result<ssh_key::PrivateKey> {
    let input = material.private_key.trim();
    let key = if input.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
        let plaintext = super::tls::decrypt_pkcs8(input, material.passphrase).map_err(|_| {
            anyhow::anyhow!("Invalid or unsupported encrypted SSH PKCS#8 key or passphrase")
        })?;
        let pem = plaintext.to_pem("PRIVATE KEY", pkcs8::LineEnding::LF)?;
        decode_secret_key(&pem, None)
    } else {
        if input.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----") {
            let parsed = ssh_key::PrivateKey::from_openssh(input)
                .map_err(|_| anyhow::anyhow!("Invalid SSH private key"))?;
            if let ssh_key::Kdf::Bcrypt { rounds, .. } = parsed.kdf() {
                ensure!(
                    *rounds > 0 && *rounds <= 128,
                    "Unsupported OpenSSH bcrypt work factor (maximum 128 rounds)"
                );
            }
        }
        decode_secret_key(input, material.passphrase.filter(|s| !s.is_empty()))
    }
    .map_err(|_| anyhow::anyhow!("Invalid SSH private key or incorrect passphrase"))?;
    valid_public(key.public_key().key_data())?;
    Ok(key)
}

fn valid_public(key: &KeyData) -> Result<()> {
    match key {
        KeyData::Ed25519(_) | KeyData::Ecdsa(_) => Ok(()),
        KeyData::Rsa(rsa) if rsa.key_size() >= 2048 => Ok(()),
        _ => bail!("Unsupported or weak SSH key algorithm"),
    }
}

pub(super) fn issuer_identity(material: &IssuerMaterial<'_>) -> Result<IssuerIdentity> {
    let key = private(material)?;
    Ok(IssuerIdentity {
        key_fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        certificate_fingerprint: None,
    })
}

pub(super) fn check_key(
    key: &str,
    passphrase: Option<&str>,
    public: &str,
) -> Result<KeyMatchReport> {
    let key = private(&IssuerMaterial {
        private_key: key,
        passphrase,
        ca_certificate: None,
    })?;
    let expected = if public
        .split_whitespace()
        .next()
        .is_some_and(|kind| kind.contains("-cert-"))
    {
        Certificate::from_openssh(public.trim())
            .map_err(|_| anyhow::anyhow!("Invalid SSH certificate"))?
            .public_key()
            .clone()
    } else {
        PublicKey::from_openssh(public.trim())
            .map_err(|_| anyhow::anyhow!("Invalid SSH public key"))?
            .key_data()
            .clone()
    };
    valid_public(&expected)?;
    Ok(KeyMatchReport {
        algorithm: key.algorithm().to_string(),
        fingerprint: key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        matches: key.public_key().key_data() == &expected,
    })
}

fn parse(input: &str) -> Result<Certificate> {
    Certificate::from_openssh(input.trim()).map_err(|_| anyhow::anyhow!("Invalid SSH certificate"))
}

fn summary(cert: &Certificate) -> Result<CertificateSummary> {
    Ok(CertificateSummary {
        kind: Some(if cert.cert_type() == CertType::User {
            CertificateKind::SshUser
        } else {
            CertificateKind::SshHost
        }),
        is_ca: false,
        identities: cert.valid_principals().to_vec(),
        serial: cert.serial().to_string(),
        issuer_fingerprint: cert
            .signature_key()
            .fingerprint(HashAlg::Sha256)
            .to_string(),
        public_key_fingerprint: cert.public_key().fingerprint(HashAlg::Sha256).to_string(),
        valid_after: i64::try_from(cert.valid_after())
            .context("SSH certificate time is out of range")?,
        valid_before: i64::try_from(cert.valid_before())
            .context("SSH certificate time is out of range")?,
    })
}

pub(super) fn inspect(input: &str) -> Result<CertificateSummary> {
    summary(&parse(input)?)
}

fn purpose(cert: &Certificate, kind: CertificateKind) -> bool {
    let right_kind = matches!(
        (kind, cert.cert_type()),
        (CertificateKind::SshUser, CertType::User) | (CertificateKind::SshHost, CertType::Host)
    );
    right_kind
        && cert.critical_options().is_empty()
        && !cert.valid_principals().is_empty()
        && cert.valid_principals().len() <= 100
        && (kind != CertificateKind::SshHost || cert.extensions().is_empty())
}

fn name_matches(cert: &Certificate, identity: &str) -> bool {
    if cert.cert_type() == CertType::Host {
        cert.valid_principals().iter().any(|name| {
            name.trim_end_matches('.')
                .eq_ignore_ascii_case(identity.trim_end_matches('.'))
        })
    } else {
        cert.valid_principals().iter().any(|name| name == identity)
    }
}

pub(super) fn verify(input: &VerifyRequest, now: i64) -> Result<VerificationReport> {
    let cert = parse(&input.certificate)?;
    let mut report = VerificationReport::new(summary(&cert)?);
    report.signature_valid = cert.verify_signature().is_ok();
    let actual = cert.signature_key().fingerprint(HashAlg::Sha256);
    for authority in &input.authorities {
        let ca = PublicKey::from_openssh(authority.trim())
            .map_err(|_| anyhow::anyhow!("Invalid trusted SSH CA public key"))?;
        valid_public(ca.key_data())?;
        if ca.fingerprint(HashAlg::Sha256) == actual {
            report.issuer_trusted = true;
        }
    }
    report.time_valid = cert.valid_after() <= now as u64 && (now as u64) < cert.valid_before();
    report.identity_matches = name_matches(&cert, &input.identity);
    report.purpose_valid = purpose(&cert, input.kind) && valid_public(cert.public_key()).is_ok();
    Ok(report.finish())
}

pub(super) fn verify_host(cert: &Certificate, host: &str, trusted: &str, now: i64) -> Result<()> {
    valid_public(cert.signature_key())?;
    ensure!(
        now >= 0 && host.len() <= 253 && !host.is_empty(),
        "Invalid SSH certificate identity or time"
    );
    ensure!(
        cert.signature_key()
            .fingerprint(HashAlg::Sha256)
            .to_string()
            == trusted,
        "SSH certificate CA is not trusted"
    );
    ensure!(
        cert.verify_signature().is_ok(),
        "SSH host certificate signature is invalid"
    );
    ensure!(
        purpose(cert, CertificateKind::SshHost),
        "SSH host certificate has invalid type or critical options"
    );
    valid_public(cert.public_key())?;
    ensure!(
        cert.valid_after() <= now as u64 && (now as u64) < cert.valid_before(),
        "SSH host certificate is expired or not yet valid"
    );
    ensure!(
        name_matches(cert, host),
        "SSH host certificate principal does not match the requested host"
    );
    Ok(())
}

pub(super) fn issue(
    material: &IssuerMaterial<'_>,
    request: &CertificateRequest,
    serial: &[u8],
    now: i64,
    end: i64,
) -> Result<IssuedCertificate> {
    let (public_key, principals, key_id, extensions, kind) = match request {
        CertificateRequest::SshUser {
            public_key,
            principals,
            key_id,
            extensions,
        } => (
            public_key,
            principals,
            key_id,
            extensions.as_slice(),
            CertType::User,
        ),
        CertificateRequest::SshHost {
            public_key,
            principals,
            key_id,
        } => (public_key, principals, key_id, &[][..], CertType::Host),
        _ => bail!("Expected an SSH certificate request"),
    };
    bounded(public_key)?;
    identities(principals)?;
    ensure!(
        key_id.len() <= 256 && !key_id.chars().any(char::is_control),
        "Invalid SSH certificate key ID"
    );
    ensure!(serial.len() <= 8, "SSH serial exceeds 64 bits");
    ensure!(
        extensions.len() <= EXTENSIONS.len()
            && extensions.iter().all(|s| EXTENSIONS.contains(&s.as_str())),
        "Unsupported SSH certificate extension"
    );
    ensure!(
        extensions
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == extensions.len(),
        "Duplicate SSH certificate extension"
    );
    if kind == CertType::Host {
        ensure!(
            principals
                .iter()
                .all(|s| !s.contains('*') && !s.contains('?') && !s.contains(char::is_whitespace)),
            "SSH host principals must be explicit host names or addresses"
        );
    }
    let ca = private(material)?;
    let public = PublicKey::from_openssh(public_key.trim())
        .map_err(|_| anyhow::anyhow!("Invalid SSH subject public key"))?;
    valid_public(public.key_data())?;
    let mut nonce = [0u8; 32];
    getrandom::fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("Certificate nonce generation failed"))?;
    let mut builder = Builder::new(
        nonce.to_vec(),
        public.key_data().clone(),
        now as u64,
        end as u64,
    )?;
    let mut serial_bytes = [0u8; 8];
    serial_bytes[8 - serial.len()..].copy_from_slice(serial);
    builder
        .serial(u64::from_be_bytes(serial_bytes))?
        .cert_type(kind)?
        .key_id(key_id.clone())?;
    for principal in principals {
        builder.valid_principal(principal.clone())?;
    }
    for extension in extensions {
        builder.extension(extension.clone(), String::new())?;
    }
    let cert = builder
        .sign(&ca)
        .map_err(|_| anyhow::anyhow!("SSH certificate signing failed"))?;
    let meta = summary(&cert)?;
    ensure!(
        cert.verify_signature().is_ok(),
        "Generated SSH certificate signature failed verification"
    );
    Ok(IssuedCertificate {
        certificate: cert.to_openssh()?,
        chain: Vec::new(),
        serial: meta.serial,
        issuer_fingerprint: meta.issuer_fingerprint,
        public_key_fingerprint: meta.public_key_fingerprint,
        valid_after: meta.valid_after,
        valid_before: meta.valid_before,
        kind: request.kind(),
    })
}
