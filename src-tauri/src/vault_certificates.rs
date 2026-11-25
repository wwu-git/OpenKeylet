use super::*;
use crate::{
    certificates::{
        self, CertificateKind, CertificateRequest, IssuedCertificate, IssuerIdentity,
        IssuerMaterial, KeyMatchReport,
    },
    model::{
        CertificateAuthorityBinding, CertificateAuthorityKind, CertificateIssuanceRecord,
        SigningGrant,
    },
};
use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy)]
pub enum CertificateActor {
    Owner,
    Agent,
}
pub enum SigningPermissionChange {
    Enable(bool),
    Allow {
        item_id: Uuid,
        kinds: Vec<CertificateKind>,
        max_ttl_seconds: u64,
    },
    Remove(Uuid),
}
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CertificateCapabilities {
    pub enabled: bool,
    pub grants: Vec<SigningGrant>,
    pub authorities: Vec<CertificateAuthorityStatus>,
}
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CertificateAuthorityStatus {
    #[schemars(with = "String")]
    pub item_id: Uuid,
    pub kind: CertificateAuthorityKind,
    pub configured: bool,
    pub effective: bool,
    pub issuer_identity: Option<IssuerIdentity>,
}

fn material<'a>(
    item: &'a StoredConnection,
    secrets: &'a SecretBundle,
    binding: &CertificateAuthorityBinding,
) -> Result<IssuerMaterial<'a>> {
    let key = item
        .modules
        .iter()
        .find(|m| m.id == binding.private_key && m.kind == "privateKey")
        .context("Certificate authority private key module unavailable")?;
    let private_key = key
        .secret_name()
        .and_then(|name| secrets.get(name))
        .context("Certificate authority private key unavailable")?;
    let ca_certificate = match binding.kind {
        CertificateAuthorityKind::Ssh => {
            anyhow::ensure!(
                binding.ca_certificate.is_none(),
                "SSH authority must not bind a TLS certificate"
            );
            None
        }
        CertificateAuthorityKind::Tls => Some(
            item.modules
                .iter()
                .find(|m| Some(m.id) == binding.ca_certificate && m.kind == "caCertificate")
                .map(|m| m.value.as_str())
                .filter(|v| !v.is_empty())
                .context("TLS CA certificate module unavailable")?,
        ),
    };
    Ok(IssuerMaterial {
        private_key,
        passphrase: module_value(&item.modules, "passphrase").or_else(|| {
            item.modules
                .iter()
                .find(|m| m.kind == "passphrase")
                .and_then(ItemModule::secret_name)
                .and_then(|name| secrets.get(name))
        }),
        ca_certificate,
    })
}
fn representative(kind: CertificateAuthorityKind) -> CertificateKind {
    match kind {
        CertificateAuthorityKind::Ssh => CertificateKind::SshUser,
        CertificateAuthorityKind::Tls => CertificateKind::TlsServer,
    }
}
fn kind_matches(binding: &CertificateAuthorityBinding, kind: CertificateKind) -> bool {
    (binding.kind == CertificateAuthorityKind::Ssh) == kind.is_ssh()
}
fn current_identity(
    item: &StoredConnection,
    secrets: &SecretBundle,
    binding: &CertificateAuthorityBinding,
) -> Result<IssuerIdentity> {
    certificates::issuer_identity(
        &material(item, secrets, binding)?,
        representative(binding.kind),
        Utc::now().timestamp(),
    )
}
fn grant_matches(
    grant: &SigningGrant,
    item: &StoredConnection,
    binding: &CertificateAuthorityBinding,
    identity: &IssuerIdentity,
) -> bool {
    grant.item_id == item.id && grant.binding == *binding && grant.issuer_identity == *identity
}

fn normalize_request(mut request: CertificateRequest) -> Result<CertificateRequest> {
    let (payload, names, metadata) = match &mut request {
        CertificateRequest::SshUser {
            public_key,
            principals,
            key_id,
            extensions,
        } => {
            anyhow::ensure!(extensions.len() <= 100, "Too many SSH extensions");
            for value in extensions.iter_mut() {
                *value = value.trim().to_owned();
            }
            extensions.sort();
            extensions.dedup();
            (public_key, principals, Some(key_id))
        }
        CertificateRequest::SshHost {
            public_key,
            principals,
            key_id,
        } => (public_key, principals, Some(key_id)),
        CertificateRequest::TlsServer { csr, names }
        | CertificateRequest::TlsClient { csr, names } => (csr, names, None),
    };
    anyhow::ensure!(
        payload.len() <= certificates::MAX_INPUT_BYTES && names.len() <= 100,
        "Certificate request is too large"
    );
    *payload = payload.trim().to_owned();
    for name in names.iter_mut() {
        anyhow::ensure!(name.len() <= 1024, "Certificate identity is too long");
        *name = name.trim().to_owned();
    }
    anyhow::ensure!(
        !names.is_empty() && names.iter().all(|name| !name.is_empty()),
        "Concrete certificate identities are required"
    );
    names.sort();
    names.dedup();
    if let Some(value) = metadata {
        anyhow::ensure!(value.len() <= 1024, "Certificate key ID is too long");
        *value = value.trim().to_owned();
    }
    Ok(request)
}

impl Vault {
    pub fn set_certificate_authority(
        &self,
        item_id: Uuid,
        binding: Option<CertificateAuthorityBinding>,
    ) -> Result<()> {
        self.update_document(|document| {
            let item = document
                .connections
                .iter_mut()
                .find(|i| i.id == item_id)
                .context("Certificate authority project unavailable")?;
            if let Some(binding) = &binding {
                current_identity(item, &self.decrypt_stored(item)?, binding)?;
            }
            if item.certificate_authority != binding {
                item.updated_at = Utc::now().to_rfc3339();
                document
                    .settings
                    .agent_signing_grants
                    .retain(|g| g.item_id != item_id);
            }
            item.certificate_authority = binding;
            Ok(())
        })
    }
    pub fn certificate_capabilities(&self) -> Result<CertificateCapabilities> {
        let document = self.read_document()?;
        let authorities = document
            .connections
            .iter()
            .filter_map(|item| {
                item.certificate_authority.as_ref().map(|binding| {
                    let identity = self
                        .decrypt_stored(item)
                        .and_then(|secrets| current_identity(item, &secrets, binding))
                        .ok();
                    let effective = document.settings.agent_signing_enabled
                        && item.enabled
                        && identity.as_ref().is_some_and(|id| {
                            document
                                .settings
                                .agent_signing_grants
                                .iter()
                                .any(|g| grant_matches(g, item, binding, id))
                        });
                    CertificateAuthorityStatus {
                        item_id: item.id,
                        kind: binding.kind,
                        configured: identity.is_some(),
                        effective,
                        issuer_identity: identity,
                    }
                })
            })
            .collect();
        Ok(CertificateCapabilities {
            enabled: document.settings.agent_signing_enabled,
            grants: document.settings.agent_signing_grants,
            authorities,
        })
    }
    pub fn change_signing_permissions(
        &self,
        change: SigningPermissionChange,
    ) -> Result<CertificateCapabilities> {
        self.update_document(|document| {
            match change {
                SigningPermissionChange::Enable(enabled) => {
                    document.settings.agent_signing_enabled = enabled
                }
                SigningPermissionChange::Remove(id) => document
                    .settings
                    .agent_signing_grants
                    .retain(|g| g.item_id != id),
                SigningPermissionChange::Allow {
                    item_id,
                    mut kinds,
                    max_ttl_seconds,
                } => {
                    anyhow::ensure!(!kinds.is_empty(), "Choose at least one certificate kind");
                    anyhow::ensure!(
                        max_ttl_seconds > 0
                            && max_ttl_seconds <= certificates::MAX_VALIDITY_SECONDS,
                        "Invalid signing TTL limit"
                    );
                    let item = document
                        .connections
                        .iter()
                        .find(|i| i.id == item_id && i.enabled)
                        .context("Certificate authority project unavailable")?;
                    let binding = item
                        .certificate_authority
                        .as_ref()
                        .context("Project is not a registered certificate authority")?;
                    anyhow::ensure!(
                        kinds.iter().all(|kind| kind_matches(binding, *kind)),
                        "Certificate kind does not match authority"
                    );
                    kinds.dedup();
                    let issuer_identity =
                        current_identity(item, &self.decrypt_stored(item)?, binding)?;
                    let grant = SigningGrant {
                        item_id,
                        binding: binding.clone(),
                        issuer_identity,
                        kinds,
                        max_ttl_seconds,
                    };
                    document
                        .settings
                        .agent_signing_grants
                        .retain(|g| g.item_id != item_id);
                    document.settings.agent_signing_grants.push(grant);
                }
            }
            Ok(())
        })?;
        self.certificate_capabilities()
    }
    pub fn guard_agent_secret_use(&self, item_id: Uuid) -> Result<()> {
        let document = self.read_document()?;
        let item = document
            .connections
            .iter()
            .find(|i| i.id == item_id)
            .context("Project unavailable")?;
        anyhow::ensure!(
            item.certificate_authority.is_none(),
            "Certificate authority secrets cannot be delegated; use certificate issuance"
        );
        Ok(())
    }
    pub fn agent_connection(&self, item_id: Uuid) -> Result<DecryptedConnection> {
        let document = self.read_document()?;
        let stored = document
            .connections
            .iter()
            .find(|i| i.id == item_id && i.enabled)
            .context("Project unavailable")?
            .clone();
        anyhow::ensure!(
            stored.certificate_authority.is_none(),
            "Certificate authority secrets cannot be delegated; use certificate issuance"
        );
        let secrets = self.decrypt_stored(&stored)?;
        Ok(DecryptedConnection { stored, secrets })
    }
    pub fn agent_secret_value(
        &self,
        item_id: Uuid,
        field: &str,
    ) -> Result<(String, String, String)> {
        let connection = self.agent_connection(item_id)?;
        let field = field.trim();
        let metadata = connection
            .secrets
            .available_fields(connection.stored.secret.as_ref())
            .into_iter()
            .find(|candidate| candidate.name == field)
            .context("This project does not have this secret field")?;
        let value = connection
            .secrets
            .get(field)
            .context("This secret field has not been configured")?
            .to_owned();
        Ok((connection.stored.name, metadata.kind, value))
    }
    pub fn check_item_key(
        &self,
        item_id: Uuid,
        key_module_id: Option<Uuid>,
        public_or_certificate: &str,
    ) -> Result<KeyMatchReport> {
        let document = self.read_document()?;
        let item = document
            .connections
            .iter()
            .find(|i| i.id == item_id)
            .context("Project unavailable")?;
        let keys: Vec<_> = item
            .modules
            .iter()
            .filter(|m| m.kind == "privateKey" && key_module_id.is_none_or(|id| id == m.id))
            .collect();
        anyhow::ensure!(keys.len() == 1, "Select exactly one private key module");
        let secrets = self.decrypt_stored(item)?;
        let key = keys[0]
            .secret_name()
            .and_then(|name| secrets.get(name))
            .context("Private key unavailable")?;
        let passphrase = item
            .modules
            .iter()
            .find(|m| m.kind == "passphrase")
            .and_then(ItemModule::secret_name)
            .and_then(|name| secrets.get(name));
        certificates::check_key(key, passphrase, public_or_certificate)
    }
    pub fn issue_certificate(
        &self,
        actor: CertificateActor,
        item_id: Uuid,
        request_id: Uuid,
        request: CertificateRequest,
        ttl_seconds: u64,
    ) -> Result<IssuedCertificate> {
        anyhow::ensure!(!request_id.is_nil(), "A nonempty request ID is required");
        anyhow::ensure!(
            ttl_seconds > 0 && ttl_seconds <= certificates::MAX_VALIDITY_SECONDS,
            "Invalid certificate TTL"
        );
        let request = normalize_request(request)?;
        let encoded = serde_json::to_vec(&(&request, ttl_seconds))?;
        anyhow::ensure!(
            encoded.len() <= certificates::MAX_INPUT_BYTES,
            "Certificate request is too large"
        );
        let digest = format!("{:x}", Sha256::digest(&encoded));
        self.update_document_with_result(|document| {
            let item = document
                .connections
                .iter()
                .find(|i| i.id == item_id && i.enabled)
                .context("Certificate authority project unavailable")?;
            let binding = item
                .certificate_authority
                .as_ref()
                .context("Project is not a registered certificate authority")?;
            anyhow::ensure!(
                kind_matches(binding, request.kind()),
                "Certificate kind does not match authority"
            );
            let grant = if matches!(actor, CertificateActor::Agent) {
                anyhow::ensure!(
                    document.settings.agent_signing_enabled,
                    "Agent certificate signing is disabled"
                );
                Some(
                    document
                        .settings
                        .agent_signing_grants
                        .iter()
                        .find(|g| {
                            g.item_id == item_id
                                && g.binding == *binding
                                && g.kinds.contains(&request.kind())
                                && ttl_seconds <= g.max_ttl_seconds
                        })
                        .context("Certificate signing permission denied")?,
                )
            } else {
                None
            };
            let secrets = self.decrypt_stored(item)?;
            let material = material(item, &secrets, binding)?;
            let now = Utc::now().timestamp();
            let identity = certificates::issuer_identity(&material, request.kind(), now)?;
            if let Some(grant) = grant {
                anyhow::ensure!(
                    grant.issuer_identity == identity,
                    "Certificate signing permission denied; authority changed"
                );
            }
            if let Some(record) = document
                .certificate_issuances
                .iter()
                .find(|r| r.issuer_item_id == item_id && r.request_id == request_id)
            {
                anyhow::ensure!(
                    record.request_digest == digest && record.issuer_identity == identity,
                    "Certificate request ID conflict"
                );
                return self.inner.key.decrypt(&record.encrypted_result);
            }
            let serial_len = if request.kind().is_ssh() { 8 } else { 16 };
            let serial = loop {
                let mut bytes = [0u8; 16];
                getrandom::fill(&mut bytes)
                    .map_err(|_| anyhow::anyhow!("Unable to generate certificate serial"))?;
                bytes[0] &= 0x7f;
                bytes[serial_len - 1] |= 1;
                let candidate = bytes[..serial_len].to_vec();
                let rendered = if request.kind().is_ssh() {
                    u64::from_be_bytes(
                        candidate
                            .as_slice()
                            .try_into()
                            .expect("eight-byte SSH serial"),
                    )
                    .to_string()
                } else {
                    candidate
                        .iter()
                        .skip_while(|byte| **byte == 0)
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<Vec<_>>()
                        .join(":")
                };
                if !document.certificate_issuances.iter().any(|record| {
                    record.issuer_identity == identity
                        && record.serial.eq_ignore_ascii_case(&rendered)
                }) {
                    break candidate;
                }
            };
            let issued =
                certificates::issue_certificate(&material, &request, ttl_seconds, &serial, now)?;
            anyhow::ensure!(
                !document
                    .certificate_issuances
                    .iter()
                    .any(|r| r.issuer_identity == identity && r.serial == issued.serial),
                "Certificate serial collision; retry request"
            );
            document
                .certificate_issuances
                .push(CertificateIssuanceRecord {
                    request_id,
                    issuer_item_id: item_id,
                    issuer_identity: identity,
                    request_digest: digest,
                    serial: issued.serial.clone(),
                    encrypted_result: self.inner.key.encrypt(&issued)?,
                    created_at: Utc::now().to_rfc3339(),
                });
            Ok(issued)
        })
    }
    pub fn set_ssh_host_ca_trust(
        &self,
        item_id: Uuid,
        host: &str,
        port: u16,
        fingerprint: Option<&str>,
    ) -> Result<()> {
        self.set_ssh_trust(item_id, host, port, SshHostTrustKind::CertificateAuthority, fingerprint)
    }
}
