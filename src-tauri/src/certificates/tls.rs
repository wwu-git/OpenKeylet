use super::*;
use anyhow::{Context, bail};
use rcgen::{
    CertificateParams, CertificateSigningRequestParams, ExtendedKeyUsagePurpose, IsCa, Issuer,
    KeyPair, KeyUsagePurpose, PublicKeyData,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime, pem::PemObject};
use std::time::Duration;
use x509_parser::{extensions::GeneralName, prelude::*};

fn cert_der(input: &str) -> Result<CertificateDer<'static>> {
    bounded(input)?;
    let input = input.trim();
    ensure!(
        input.starts_with("-----BEGIN CERTIFICATE-----")
            && input.ends_with("-----END CERTIFICATE-----")
            && input.matches("-----BEGIN CERTIFICATE-----").count() == 1,
        "Expected exactly one PEM certificate without additional material"
    );
    CertificateDer::from_pem_slice(input.as_bytes())
        .map_err(|_| anyhow::anyhow!("Invalid PEM certificate"))
}

fn parse(der: &[u8]) -> Result<X509Certificate<'_>> {
    let (rest, cert) =
        X509Certificate::from_der(der).map_err(|_| anyhow::anyhow!("Invalid X.509 certificate"))?;
    ensure!(rest.is_empty(), "Trailing certificate data");
    Ok(cert)
}

fn private(input: &str, password: Option<&str>) -> Result<KeyPair> {
    let result = (|| -> Result<KeyPair> {
        let input = input.trim();
        if input.starts_with("-----BEGIN RSA PRIVATE KEY-----") {
            use rsa::{pkcs1::DecodeRsaPrivateKey, pkcs8::EncodePrivateKey};
            let key = rsa::PrivateKey::from_pkcs1_pem(input)?;
            let der = key.to_pkcs8_der()?;
            return Ok(KeyPair::try_from(der.as_bytes())?);
        }
        if input.starts_with("-----BEGIN EC PRIVATE KEY-----") {
            use p256::pkcs8::EncodePrivateKey;
            let key = p256::SecretKey::from_sec1_pem(input)?;
            let der = key.to_pkcs8_der()?;
            return Ok(KeyPair::try_from(der.as_bytes())?);
        }
        if input.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----") {
            let plaintext = decrypt_pkcs8(input, password)?;
            return Ok(KeyPair::try_from(plaintext.as_bytes())?);
        }
        let (label, plaintext) = pkcs8::SecretDocument::from_pem(input)?;
        ensure!(label == "PRIVATE KEY", "Unsupported private key PEM label");
        Ok(KeyPair::try_from(plaintext.as_bytes())?)
    })();
    let key=result.map_err(|_|anyhow::anyhow!("Unsupported or invalid TLS private key, encryption, or passphrase (PKCS#8, RSA PKCS#1, or P-256 SEC1 required)"))?;
    valid_spki(&key.subject_public_key_info())?;
    Ok(key)
}

pub(super) fn decrypt_pkcs8(input: &str, password: Option<&str>) -> Result<pkcs8::SecretDocument> {
    use pkcs8::pkcs5::{EncryptionScheme, pbes2::Kdf};
    let (_, encrypted) = pkcs8::SecretDocument::from_pem(input)?;
    let info = pkcs8::EncryptedPrivateKeyInfoRef::try_from(encrypted.as_bytes())?;
    // Bound attacker-controlled KDF work before handing the ciphertext to the library.
    match &info.encryption_algorithm {
        EncryptionScheme::Pbes2(params) => match &params.kdf {
            Kdf::Pbkdf2(kdf) => ensure!(
                kdf.iteration_count > 0 && kdf.iteration_count <= 2_000_000,
                "Unsupported encrypted key work factor"
            ),
            Kdf::Scrypt(kdf) => ensure!(
                kdf.cost_parameter > 1
                    && kdf.cost_parameter <= 32768
                    && kdf.block_size > 0
                    && kdf.block_size <= 8
                    && kdf.parallelization > 0
                    && kdf.parallelization <= 4,
                "Unsupported encrypted key work factor"
            ),
            _ => bail!("Unsupported encrypted key derivation"),
        },
        _ => bail!("Unsupported encrypted key format"),
    }
    let password = password.context("Passphrase required")?;
    Ok(info.decrypt(password)?)
}

fn valid_spki(der: &[u8]) -> Result<()> {
    let (rest, spki) =
        SubjectPublicKeyInfo::from_der(der).map_err(|_| anyhow::anyhow!("Invalid public key"))?;
    ensure!(rest.is_empty(), "Trailing public key data");
    use x509_parser::public_key::PublicKey;
    match spki
        .parsed()
        .map_err(|_| anyhow::anyhow!("Invalid public key"))?
    {
        PublicKey::RSA(rsa) => ensure!(
            rsa.key_size() >= 2048,
            "RSA keys must be at least 2048 bits"
        ),
        PublicKey::EC(_) => ensure!(
            matches!(
                spki.algorithm
                    .parameters
                    .as_ref()
                    .and_then(|p| p.as_oid().ok())
                    .map(|o| o.to_id_string())
                    .as_deref(),
                Some("1.2.840.10045.3.1.7" | "1.3.132.0.34")
            ),
            "Unsupported EC curve"
        ),
        PublicKey::Unknown(_) if spki.algorithm.algorithm.to_id_string() == "1.3.101.112" => {}
        _ => bail!("Unsupported public key algorithm"),
    }
    Ok(())
}

fn ca<'a>(der: &'a [u8], key: &KeyPair, now: i64) -> Result<X509Certificate<'a>> {
    let cert = parse(der)?;
    ensure!(
        cert.public_key().raw == key.subject_public_key_info(),
        "CA certificate and private key do not match"
    );
    ensure!(
        cert.basic_constraints()?.is_some_and(|b| b.value.ca),
        "Issuer certificate is not a CA"
    );
    ensure!(
        cert.key_usage()?.is_none_or(|u| u.value.key_cert_sign()),
        "Issuer certificate does not permit certificate signing"
    );
    ensure!(
        cert.validity().not_before.timestamp() <= now
            && now < cert.validity().not_after.timestamp(),
        "Issuer certificate is expired or not yet valid"
    );
    Ok(cert)
}

pub(super) fn issuer_identity(material: &IssuerMaterial<'_>, now: i64) -> Result<IssuerIdentity> {
    let key = private(material.private_key, material.passphrase)?;
    let der = cert_der(
        material
            .ca_certificate
            .context("TLS signing requires an explicit CA certificate")?,
    )?;
    ca(&der, &key, now)?;
    Ok(IssuerIdentity {
        key_fingerprint: fingerprint(&key.subject_public_key_info()),
        certificate_fingerprint: Some(fingerprint(&der)),
    })
}

pub(super) fn check_key(
    input: &str,
    password: Option<&str>,
    public: &str,
) -> Result<KeyMatchReport> {
    let key = private(input, password)?;
    let expected = if public
        .trim_start()
        .starts_with("-----BEGIN CERTIFICATE-----")
    {
        let der = cert_der(public)?;
        parse(&der)?.public_key().raw.to_vec()
    } else {
        let (label, doc) = pkcs8::Document::from_pem(public)
            .map_err(|_| anyhow::anyhow!("Invalid PEM public key"))?;
        ensure!(
            label == "PUBLIC KEY",
            "Expected an SPKI PUBLIC KEY or certificate"
        );
        doc.as_bytes().to_vec()
    };
    valid_spki(&expected)?;
    Ok(KeyMatchReport {
        algorithm: format!("{:?}", key.algorithm()),
        fingerprint: fingerprint(&key.subject_public_key_info()),
        matches: key.subject_public_key_info() == expected,
    })
}

fn summary(cert: &X509Certificate<'_>) -> Result<CertificateSummary> {
    let is_ca = cert.basic_constraints()?.is_some_and(|b| b.value.ca);
    let kind = if is_ca {
        None
    } else {
        cert.extended_key_usage()?.and_then(|eku| {
            match (eku.value.server_auth, eku.value.client_auth, eku.value.any) {
                (true, false, false) => Some(CertificateKind::TlsServer),
                (false, true, false) => Some(CertificateKind::TlsClient),
                _ => None,
            }
        })
    };
    let names = cert
        .subject_alternative_name()?
        .map(|san| {
            san.value
                .general_names
                .iter()
                .filter_map(|n| match n {
                    GeneralName::DNSName(s) => Some(s.to_string()),
                    GeneralName::IPAddress(bytes) => match bytes.len() {
                        4 => Some(
                            std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3])
                                .to_string(),
                        ),
                        16 => Some(
                            std::net::Ipv6Addr::from(<[u8; 16]>::try_from(*bytes).ok()?)
                                .to_string(),
                        ),
                        _ => None,
                    },
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(CertificateSummary {
        kind,
        is_ca,
        identities: names,
        serial: cert.raw_serial_as_string(),
        issuer_fingerprint: String::new(),
        public_key_fingerprint: fingerprint(cert.public_key().raw),
        valid_after: cert.validity().not_before.timestamp(),
        valid_before: cert.validity().not_after.timestamp(),
    })
}

pub(super) fn inspect(input: &str) -> Result<CertificateSummary> {
    let der = cert_der(input)?;
    summary(&parse(&der)?)
}

pub(super) fn issue(
    material: &IssuerMaterial<'_>,
    request: &CertificateRequest,
    serial: &[u8],
    now: i64,
    end: i64,
) -> Result<IssuedCertificate> {
    let (csr, names) = match request {
        CertificateRequest::TlsServer { csr, names }
        | CertificateRequest::TlsClient { csr, names } => (csr, names),
        _ => bail!("Expected TLS certificate request"),
    };
    bounded(csr)?;
    identities(names)?;
    ensure!(
        serial.len() <= 20 && serial[0] & 0x80 == 0,
        "TLS serial must be a positive integer of at most 20 octets"
    );
    let key = private(material.private_key, material.passphrase)?;
    let ca_pem = material
        .ca_certificate
        .context("TLS signing requires an explicit CA certificate")?;
    let ca_der = cert_der(ca_pem)?;
    let ca_cert = ca(&ca_der, &key, now)?;
    ensure!(
        end <= ca_cert.validity().not_after.timestamp(),
        "Issued certificate cannot outlive the CA"
    );
    ensure!(
        ca_cert.extended_key_usage()?.is_none_or(|e| e.value.any
            || if request.kind() == CertificateKind::TlsServer {
                e.value.server_auth
            } else {
                e.value.client_auth
            }),
        "CA certificate does not permit the requested TLS purpose"
    );
    let csr = CertificateSigningRequestParams::from_pem(csr)
        .map_err(|_| anyhow::anyhow!("Invalid or unsupported CSR, extensions, or signature"))?;
    valid_spki(&csr.public_key.subject_public_key_info())?;
    let mut params = CertificateParams::new(names.clone())
        .map_err(|_| anyhow::anyhow!("Invalid TLS identity"))?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params.is_ca = IsCa::ExplicitNoCa;
    params.not_before = ::time::OffsetDateTime::from_unix_timestamp(now)?;
    params.not_after = ::time::OffsetDateTime::from_unix_timestamp(end)?;
    params.serial_number = Some(serial.to_vec().into());
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![if request.kind() == CertificateKind::TlsServer {
        ExtendedKeyUsagePurpose::ServerAuth
    } else {
        ExtendedKeyUsagePurpose::ClientAuth
    }];
    params.use_authority_key_identifier_extension = true;
    let issuer = Issuer::from_ca_cert_pem(ca_pem, key)
        .map_err(|_| anyhow::anyhow!("Unsupported CA certificate"))?;
    let cert = params
        .signed_by(&csr.public_key, &issuer)
        .map_err(|_| anyhow::anyhow!("TLS certificate signing failed"))?;
    let certificate = cert.pem();
    let leaf = parse(cert.der())?;
    for name in names {
        let report = verify(
            &VerifyRequest {
                certificate: certificate.clone(),
                authorities: vec![ca_pem.to_owned()],
                intermediates: Vec::new(),
                identity: name.clone(),
                kind: request.kind(),
            },
            now,
        )?;
        ensure!(
            report.verified,
            "Issued certificate does not satisfy issuer constraints, identity, or purpose"
        );
    }
    ensure!(
        leaf.verify_signature(Some(ca_cert.public_key())).is_ok(),
        "Generated TLS certificate failed signature verification"
    );
    Ok(IssuedCertificate {
        certificate,
        chain: vec![
            pkcs8::Document::try_from(ca_der.as_ref())?
                .to_pem("CERTIFICATE", pkcs8::LineEnding::LF)?,
        ],
        serial: leaf.raw_serial_as_string(),
        issuer_fingerprint: fingerprint(ca_cert.public_key().raw),
        public_key_fingerprint: fingerprint(leaf.public_key().raw),
        valid_after: now,
        valid_before: end,
        kind: request.kind(),
    })
}

pub(super) fn verify(input: &VerifyRequest, now: i64) -> Result<VerificationReport> {
    let der = cert_der(&input.certificate)?;
    let cert = parse(&der)?;
    let mut report = VerificationReport::new(summary(&cert)?);
    let authorities = input
        .authorities
        .iter()
        .map(|s| cert_der(s))
        .collect::<Result<Vec<_>>>()?;
    let intermediates = input
        .intermediates
        .iter()
        .map(|s| cert_der(s))
        .collect::<Result<Vec<_>>>()?;
    for authority in &authorities {
        let ca = parse(authority)?;
        ensure!(
            ca.basic_constraints()?.is_some_and(|b| b.value.ca)
                && ca.key_usage()?.is_none_or(|u| u.value.key_cert_sign()),
            "Trusted TLS authority is not a certificate-signing CA"
        );
        ensure!(
            ca.validity().not_before.timestamp() <= now
                && now < ca.validity().not_after.timestamp(),
            "Trusted TLS authority is expired or not yet valid"
        );
        ensure!(
            ca.extended_key_usage()?.is_none_or(|e| e.value.any
                || if input.kind == CertificateKind::TlsServer {
                    e.value.server_auth
                } else {
                    e.value.client_auth
                }),
            "Trusted TLS authority does not permit the requested usage"
        );
    }
    let anchors = authorities
        .iter()
        .map(webpki::anchor_from_trusted_cert)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| anyhow::anyhow!("Invalid trusted TLS CA certificate"))?;
    let ee = webpki::EndEntityCert::try_from(&der)
        .map_err(|_| anyhow::anyhow!("Invalid TLS leaf certificate"))?;
    let name = ServerName::try_from(input.identity.as_str())
        .map_err(|_| anyhow::anyhow!("TLS identity must be a DNS name or IP address"))?;
    report.identity_matches = ee.verify_is_valid_for_subject_name(&name).is_ok();
    let eku = cert.extended_key_usage()?;
    report.purpose_valid = !report.summary.as_ref().is_some_and(|s| s.is_ca)
        && eku.is_some_and(|e| {
            if input.kind == CertificateKind::TlsServer {
                e.value.server_auth
            } else {
                e.value.client_auth
            }
        })
        && cert
            .key_usage()?
            .is_none_or(|u| u.value.digital_signature())
        && valid_spki(cert.public_key().raw).is_ok();
    report.time_valid = cert.validity().not_before.timestamp() <= now
        && now < cert.validity().not_after.timestamp();
    let usage = if input.kind == CertificateKind::TlsServer {
        webpki::KeyUsage::server_auth()
    } else {
        webpki::KeyUsage::client_auth()
    };
    let path = ee.verify_for_usage(
        webpki::ALL_VERIFICATION_ALGS,
        &anchors,
        &intermediates,
        UnixTime::since_unix_epoch(Duration::from_secs(now as u64)),
        usage,
        None,
        None,
    );
    if path.is_ok() {
        report.signature_valid = true;
        report.issuer_trusted = true;
        if let Some(issuer) = intermediates
            .iter()
            .chain(authorities.iter())
            .filter_map(|der| parse(der).ok())
            .find(|issuer| {
                cert.issuer() == issuer.subject()
                    && cert.verify_signature(Some(issuer.public_key())).is_ok()
            })
        {
            if let Some(summary) = report.summary.as_mut() {
                summary.issuer_fingerprint = fingerprint(issuer.public_key().raw);
            }
        }
    } else {
        for issuer in intermediates.iter().chain(authorities.iter()) {
            let issuer = parse(issuer)?;
            if cert.issuer() == issuer.subject()
                && cert.verify_signature(Some(issuer.public_key())).is_ok()
            {
                report.signature_valid = true;
            }
        }
        report.reasons.push("TLS certification path validation failed (including CA constraints, validity, signatures, and requested usage)".into());
    }
    Ok(report.finish())
}
