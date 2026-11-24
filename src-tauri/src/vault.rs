use crate::{
    api_catalog,
    crypto::{MasterKey, create_owner_pin_verifier, verify_owner_pin},
    model::{
        Activity, AppState, ConnectionInput, ImportSummary, ItemModule, McpState, NewActivity,
        OwnerEditorDraft, OwnerSecretField, OwnerSecretView, PortableConnection, PublicConnection,
        SecretBundle, SecretField, SecretProfile, SecurityState, Settings, SettingsPatch,
        StoredConnection, StoredEditorDraft, VaultDocument, module_kind_has_plaintext_reveal,
    },
};
use anyhow::{Context, Result, bail};
use atomic_write_file::AtomicWriteFile;
use chrono::Utc;
use fs2::FileExt;
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use url::Url;
use uuid::Uuid;

const MAX_ACTIVITIES: usize = 3_000;
const VAULT_VERSION: u8 = 18;
#[path = "vault_certificates.rs"]
mod certificates;
pub use certificates::{CertificateActor, CertificateCapabilities, CertificateAuthorityStatus, SigningPermissionChange};
#[cfg(test)]
#[path = "oci_module_tests.rs"]
mod oci_module_tests;
#[cfg(test)]
#[path = "vault_lifecycle_tests.rs"]
mod lifecycle_tests;
#[cfg(test)]
#[path = "ssh_jump_tests.rs"]
mod ssh_jump_tests;
#[cfg(test)]
#[path = "multi_address_tests.rs"]
mod multi_address_tests;

#[cfg(test)]
#[path = "vault_provider_tests.rs"]
mod provider_tests;

#[cfg(test)]
#[path = "vault_certificate_tests.rs"]
mod certificate_tests;

#[derive(Clone)]
pub struct Vault {
    inner: Arc<VaultInner>,
}

struct VaultInner {
    data_dir: PathBuf,
    vault_path: PathBuf,
    lock_path: PathBuf,
    key: MasterKey,
}

pub enum WritePermissionChange<'a> {
    Enable(bool),
    AddFolder(&'a str),
    RemoveFolder(&'a str),
}

pub struct DecryptedConnection {
    pub stored: StoredConnection,
    pub secrets: SecretBundle,
}

impl Vault {
    pub fn open(data_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&data_dir).context("Unable to create vault directory")?;
        let bootstrap = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data_dir.join("bootstrap.lock"))
            .context("Could not open vault startup lock")?;
        FileExt::lock_exclusive(&bootstrap).context("Could not lock vault startup process")?;

        let result = (|| {
            let vault_path = data_dir.join("vault.json");
            let key = MasterKey::load_or_create(&data_dir, vault_path.exists())?;
            let vault = Self {
                inner: Arc::new(VaultInner {
                    lock_path: data_dir.join("vault.lock"),
                    data_dir,
                    vault_path,
                    key,
                }),
            };

            if !vault.inner.vault_path.exists() {
                let document = VaultDocument {
                    version: VAULT_VERSION,
                    certificate_issuances: Vec::new(),
                    ssh_host_trust: Vec::new(),
                    settings: Settings::default(),
                    browser_bridge_secret: None,
                    owner_pin: None,
                    connections: Vec::new(),
                    trash: Vec::new(),
                    editor_drafts: Vec::new(),
                    activities: Vec::new(),
                };
                vault.with_exclusive_lock(|_| vault.write_document_unlocked(&document))?;
            } else {
                vault.require_supported_version()?;
                vault.migrate_oci_modules()?;
            }
            Ok(vault)
        })();
        let _ = FileExt::unlock(&bootstrap);
        result
    }

    pub fn ssh_host_trust(&self) -> Result<Vec<crate::model::SshHostTrust>> {
        Ok(self.read_document()?.ssh_host_trust)
    }

    pub fn set_ssh_host_trust(&self, item_id: Uuid, host: &str, port: u16, fingerprint: Option<&str>) -> Result<()> {
        let host = ssh_endpoint_host(host)?;
        anyhow::ensure!(port > 0, "Invalid SSH port");
        if let Some(value) = fingerprint {
            use base64::Engine;
            let encoded = value.strip_prefix("SHA256:").context("Expected SHA256 SSH fingerprint")?;
            let decoded = base64::engine::general_purpose::STANDARD_NO_PAD.decode(encoded)
                .context("Invalid SHA256 SSH fingerprint")?;
            anyhow::ensure!(decoded.len() == 32, "Invalid SHA256 SSH fingerprint");
        }
        self.update_document(|document| {
            anyhow::ensure!(document.connections.iter().any(|item| item.id == item_id), "SSH project unavailable");
            document.ssh_host_trust.retain(|entry| entry.item_id != item_id || entry.host != host || entry.port != port);
            if let Some(fingerprint) = fingerprint {
                document.ssh_host_trust.push(crate::model::SshHostTrust { kind: crate::model::SshHostTrustKind::HostKey, item_id, host, port, fingerprint: fingerprint.to_owned() });
            }
            Ok(())
        })
    }

    pub(crate) fn validate_ssh_jump(&self, document: &VaultDocument, item: &StoredConnection, secrets: &SecretBundle) -> Result<()> {
        anyhow::ensure!(!item.ssh_jump_unresolved, "SSH jump reference is unresolved; select the jump project again");
        if let Some(id) = item.ssh_jump_item_id {
            anyhow::ensure!(id != item.id, "An SSH project cannot be its own jump host");
            anyhow::ensure!(ssh_config_complete(item, secrets), "SSH target configuration is incomplete");
            let jump = document.connections.iter().find(|candidate| candidate.id == id)
                .context("SSH jump project is unavailable")?;
            anyhow::ensure!(jump.enabled && jump.ssh_jump_item_id.is_none() && !jump.ssh_jump_unresolved, "SSH jump must be enabled and use a direct connection");
            let jump_secrets = self.decrypt_stored(jump)?;
            anyhow::ensure!(ssh_config_complete(jump, &jump_secrets), "SSH jump configuration is incomplete");
        }
        if item.ssh_jump_item_id.is_some() {
            anyhow::ensure!(!document.connections.iter().any(|candidate| candidate.id != item.id && candidate.ssh_jump_item_id == Some(item.id)), "Multi-hop SSH is not supported");
        }
        Ok(())
    }

    pub fn ssh_jump_connection(&self, target: &DecryptedConnection) -> Result<Option<DecryptedConnection>> {
        let document = self.read_document()?;
        self.validate_ssh_jump(&document, &target.stored, &target.secrets)?;
        target.stored.ssh_jump_item_id.map(|id| {
            let stored = document.connections.iter().find(|item| item.id == id).context("SSH jump project unavailable")?.clone();
            let secrets = self.decrypt_stored(&stored)?;
            Ok(DecryptedConnection { stored, secrets })
        }).transpose()
    }

    pub fn data_dir(&self) -> &Path {
        &self.inner.data_dir
    }

    pub fn storage_label(&self) -> &str {
        self.inner.key.source()
    }

    // Device-local integration credentials never enter portable/cloud exports.
    pub(crate) fn protect_local_state<T: serde::Serialize>(
        &self,
        value: &T,
    ) -> Result<crate::model::SecretEnvelope> {
        self.inner.key.encrypt(value)
    }

    pub(crate) fn unprotect_local_state<T: serde::de::DeserializeOwned>(
        &self,
        envelope: &crate::model::SecretEnvelope,
    ) -> Result<T> {
        self.inner.key.decrypt(envelope)
    }

    fn require_supported_version(&self) -> Result<()> {
        let bytes = fs::read(&self.inner.vault_path).context("Could not read vault")?;
        let document: VaultDocument =
            serde_json::from_slice(&bytes).context("Invalid vault file format")?;
        if !matches!(document.version, 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | VAULT_VERSION) {
            bail!(
                "OpenKeylet supports vault versions 7 through 18; current file version is {}",
                document.version
            );
        }
        Ok(())
    }

    fn migrate_oci_modules(&self) -> Result<()> {
        self.with_exclusive_lock(|_| {
            let raw: VaultDocument = serde_json::from_slice(&fs::read(&self.inner.vault_path)?)?;
            if raw.version == VAULT_VERSION {
                return Ok(());
            }
            let mut document = self.read_document_unlocked()?;
            if matches!(raw.version, 16 | 17) {
                document.version = VAULT_VERSION;
                return self.write_document_unlocked(&document);
            }
            for stored in document
                .connections
                .iter_mut()
                .chain(document.trash.iter_mut().map(|entry| &mut entry.connection))
            {
                let secrets: SecretBundle = self.inner.key.decrypt(&stored.encrypted_secrets)?;
                if secrets.oci.is_none() {
                    continue;
                }
                let mut input: ConnectionInput =
                    serde_json::from_value(serde_json::to_value(stored.portable())?)?;
                input.secrets = secrets;
                migrate_legacy_oci_input(&mut input)?;
                stored.modules = input.modules;
                stored.oci_bindings = input.oci_bindings;
                stored.secret = Some(SecretProfile {
                    fields: module_fields(&stored.modules),
                });
                stored.encrypted_secrets = self.inner.key.encrypt(&input.secrets)?;
            }
            let backup = self
                .inner
                .data_dir
                .join(format!("before-oci-modules-{}", Uuid::new_v4()));
            fs::create_dir(&backup).context("Could not create OCI migration backup")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&backup, fs::Permissions::from_mode(0o700))?;
            }
            fs::copy(&self.inner.vault_path, backup.join("vault.json"))
                .context("Could not back up vault before OCI migration")?;
            fs::copy(
                self.inner.data_dir.join("master.key"),
                backup.join("master.key"),
            )
            .context("Could not back up master key before OCI migration")?;
            File::open(backup.join("vault.json"))?.sync_all()?;
            File::open(backup.join("master.key"))?.sync_all()?;
            document.version = VAULT_VERSION;
            self.write_document_unlocked(&document)
        })
    }
    pub fn owner_pin_configured(&self) -> Result<bool> {
        Ok(self.read_document()?.owner_pin.is_some())
    }

    pub fn set_owner_pin(&self, pin: &str) -> Result<()> {
        let verifier = create_owner_pin_verifier(pin)?;
        self.update_document(|document| {
            if document.owner_pin.is_some() {
                bail!("PIN is set");
            }
            document.owner_pin = Some(verifier);
            Ok(())
        })
    }

    pub fn disable_owner_pin(&self) -> Result<()> {
        self.update_document(|document| {
            document.owner_pin = None;
            Ok(())
        })
    }

    pub fn verify_owner_pin(&self, pin: &str) -> Result<bool> {
        let document = self.read_document()?;
        let verifier = document
            .owner_pin
            .as_ref()
            .context("PIN has not been set")?;
        verify_owner_pin(pin, verifier)
    }

    pub fn owner_secret_view(&self, id: Uuid) -> Result<OwnerSecretView> {
        let connection = self.get_connection(id)?;
        Ok(Self::owner_secret_view_for(&connection))
    }

    pub(crate) fn owner_secret_view_for(connection: &DecryptedConnection) -> OwnerSecretView {
        let fields = connection
            .secrets
            .available_fields(connection.stored.secret.as_ref())
            .into_iter()
            .filter_map(|field| {
                connection
                    .secrets
                    .get(&field.name)
                    .map(|value| OwnerSecretField {
                        name: field.name,
                        kind: field.kind,
                        value: value.to_owned(),
                    })
            })
            .collect();
        OwnerSecretView {
            id: connection.stored.id,
            fields,
        }
    }

    pub fn list_editor_drafts(&self) -> Result<Vec<OwnerEditorDraft>> {
        let document = self.read_document()?;
        let mut drafts = document
            .editor_drafts
            .into_iter()
            .map(|draft| {
                Ok(OwnerEditorDraft {
                    id: draft.id,
                    updated_at: draft.updated_at,
                    input: self.inner.key.decrypt(&draft.payload)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        drafts.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        drafts.truncate(1);
        Ok(drafts)
    }

    pub fn save_editor_draft(
        &self,
        id: Option<Uuid>,
        mut input: ConnectionInput,
    ) -> Result<OwnerEditorDraft> {
        input.secrets.oci = None;
        input.name = checked_text(&input.name, "Project name", 80)?;
        input.description = checked_text(&input.description, "Project description", 240)?;
        let id = id.unwrap_or_else(Uuid::new_v4);
        let updated_at = Utc::now().to_rfc3339();
        let payload = self.inner.key.encrypt(&input)?;
        self.update_document(|document| {
            document.editor_drafts.clear();
            document.editor_drafts.push(StoredEditorDraft {
                id,
                updated_at: updated_at.clone(),
                payload,
            });
            Ok(())
        })?;
        Ok(OwnerEditorDraft {
            id,
            updated_at,
            input,
        })
    }

    pub fn delete_editor_draft(&self, id: Uuid) -> Result<()> {
        self.update_document(|document| {
            document.editor_drafts.retain(|draft| draft.id != id);
            Ok(())
        })
    }

    pub fn settings(&self) -> Result<Settings> {
        Ok(self.read_document()?.settings)
    }

    pub fn update_settings(&self, mut next: Settings) -> Result<Settings> {
        if next.browser_port == 0 {
            bail!("Local port must be between 1–65535");
        }
        next.language = if next.language.eq_ignore_ascii_case("en") {
            "en".to_owned()
        } else {
            "zh".to_owned()
        };
        next.close_behavior = if next.close_behavior.eq_ignore_ascii_case("exit") {
            "exit".to_owned()
        } else {
            "tray".to_owned()
        };
        next.agent_write_folders = crate::vault_write::normalize_folders(next.agent_write_folders)?;
        self.update_document(|document| {
            // Pairing state is changed only by the authenticated bridge handshake.
            next.browser_paired = document.settings.browser_paired;
            document.settings = next.clone();
            Ok(())
        })?;
        Ok(next)
    }

    pub fn change_write_permissions(&self, change: WritePermissionChange<'_>) -> Result<Settings> {
        use crate::vault_write::normalize_folders;
        self.update_document_with_result(|document| {
            let settings = &mut document.settings;
            match change {
                WritePermissionChange::Enable(enabled) => {
                    if enabled && settings.agent_write_folders.is_empty() {
                        bail!("Add at least one writable folder before enabling MCP writes");
                    }
                    settings.agent_write_enabled = enabled;
                }
                WritePermissionChange::AddFolder(name) => {
                    let mut folders = settings.agent_write_folders.clone();
                    folders.push(name.to_owned());
                    settings.agent_write_folders = normalize_folders(folders)?;
                }
                WritePermissionChange::RemoveFolder(name) => {
                    let name = normalize_folders(vec![name.to_owned()])?.remove(0);
                    let index = settings.agent_write_folders.iter().position(|folder| folder == &name)
                        .context("Writable folder not found")?;
                    settings.agent_write_folders.remove(index);
                }
            }
            Ok(settings.clone())
        })
    }

    pub fn update_settings_patch(&self, patch: SettingsPatch) -> Result<Settings> {
        self.update_document_with_result(|document| {
            let mut next = document.settings.clone();
            if let Some(enabled) = patch.agent_write_enabled { next.agent_write_enabled = enabled; }
            if let Some(folders) = patch.agent_write_folders { next.agent_write_folders = crate::vault_write::normalize_folders(folders)?; }
            if let Some(language) = patch.language {
                next.language = if language.eq_ignore_ascii_case("en") {
                    "en".to_owned()
                } else {
                    "zh".to_owned()
                };
            }
            if let Some(close_behavior) = patch.close_behavior {
                next.close_behavior = if close_behavior.eq_ignore_ascii_case("exit") {
                    "exit".to_owned()
                } else {
                    "tray".to_owned()
                };
            }
            if let Some(browser_enabled) = patch.browser_enabled {
                next.browser_enabled = browser_enabled;
            }
            if let Some(browser_port) = patch.browser_port {
                if browser_port == 0 {
                    bail!("Local port must be between 1–65535");
                }
                next.browser_port = browser_port;
            }
            document.settings = next.clone();
            Ok(next)
        })
    }

    pub fn browser_bridge_secret(&self) -> Result<String> {
        if let Some(secret) = self.read_document()?.browser_bridge_secret {
            return self.inner.key.decrypt(&secret);
        }
        self.with_exclusive_lock(|_| {
            let mut document = self.read_document_unlocked()?;
            if let Some(secret) = document.browser_bridge_secret.as_ref() {
                return self.inner.key.decrypt(secret);
            }
            let secret = random_token()?;
            document.browser_bridge_secret = Some(self.inner.key.encrypt(&secret)?);
            self.write_document_unlocked(&document)?;
            Ok(secret)
        })
    }

    pub fn rotate_browser_bridge_secret(&self) -> Result<()> {
        let encrypted = self.inner.key.encrypt(&random_token()?)?;
        self.update_document(|document| {
            document.browser_bridge_secret = Some(encrypted);
            document.settings.browser_paired = false;
            Ok(())
        })
    }

    pub fn set_browser_paired(&self, paired: bool) -> Result<()> {
        self.update_document(|document| {
            document.settings.browser_paired = paired;
            Ok(())
        })
    }

    pub fn list_connections(&self) -> Result<Vec<PublicConnection>> {
        self.list_decrypted_connections().map(|connections| {
            connections
                .into_iter()
                .map(|connection| connection.stored.public(Some(&connection.secrets)))
                .collect()
        })
    }

    pub fn list_decrypted_connections(&self) -> Result<Vec<DecryptedConnection>> {
        let document = self.read_document()?;
        document
            .connections
            .into_iter()
            .map(|stored| {
                let secrets = self.inner.key.decrypt(&stored.encrypted_secrets)?;
                Ok(DecryptedConnection { stored, secrets })
            })
            .collect()
    }

    pub fn get_connection(&self, id: Uuid) -> Result<DecryptedConnection> {
        let document = self.read_document()?;
        let stored = document
            .connections
            .into_iter()
            .find(|connection| connection.id == id)
            .context("Connection not found")?;
        let secrets = self.inner.key.decrypt(&stored.encrypted_secrets)?;
        Ok(DecryptedConnection { stored, secrets })
    }

    pub fn get_secret_value(&self, id: Uuid, field: &str) -> Result<(String, String, String)> {
        let connection = self.get_connection(id)?;
        if !connection.stored.enabled {
            bail!("This project is disabled");
        }
        // Module names are length-checked when saved. Do not truncate a caller's
        // lookup here: a longer, different name must never alias an existing
        // credential and fill the wrong value.
        let field = field.trim().to_owned();
        let metadata = connection
            .secrets
            .available_fields(connection.stored.secret.as_ref())
            .into_iter()
            .find(|candidate| candidate.name == field)
            .context("This project does not have this secret field")?;
        let value = connection
            .secrets
            .get(&field)
            .context("This secret field has not been configured")?
            .to_owned();
        Ok((connection.stored.name, metadata.kind, value))
    }

    pub fn save_connection(&self, input: ConnectionInput) -> Result<PublicConnection> {
        self.save_connection_checked(input, None)
    }

    pub fn save_connection_checked(
        &self,
        input: ConnectionInput,
        expected_updated_at: Option<&str>,
    ) -> Result<PublicConnection> {
        self.save_connection_with_private_key(input, expected_updated_at, None)
    }

    pub(crate) fn save_connection_with_private_key(
        &self,
        input: ConnectionInput,
        expected_updated_at: Option<&str>,
        private_key: Option<(&str, &str)>,
    ) -> Result<PublicConnection> {
        self.save_connection_inner(input, expected_updated_at, private_key)
    }

    #[cfg(test)]
    pub(crate) fn save_oci_connection(
        &self,
        mut input: ConnectionInput,
        expected: Option<&str>,
        config: crate::oci::Config,
        key: Option<&str>,
        _ssh_key: Option<(&str, &str)>,
    ) -> Result<PublicConnection> {
        let key = if let Some(key) = key {
            key.to_owned()
        } else {
            let id = input.id.context("Import an OCI signing private key")?;
            let old = self.get_connection(id)?;
            crate::oci::Credential::from_modules(&old.stored, &old.secrets)?
                .key_pem
                .as_deref()
                .unwrap_or_default()
                .to_owned()
        };
        input.secrets.oci = Some(crate::oci::Credential::new(config.clone(), &key)?);
        input.oci_bindings = None;
        input.http_auth_type = crate::oci::AUTH_MODE.to_owned();
        input.item_type = "api".to_owned();
        input.modules.retain(|module| module.kind != "url");
        push_module(&mut input.modules, "url", "", config.base_url()?.as_str());
        migrate_legacy_oci_input(&mut input)?;
        self.save_connection_inner(input, expected, None)
    }

    fn save_connection_inner(
        &self,
        input: ConnectionInput,
        expected_updated_at: Option<&str>,
        private_key: Option<(&str, &str)>,
    ) -> Result<PublicConnection> {
        self.update_document_with_result(|document| {
            self.save_connection_in_document(document, input, expected_updated_at, private_key)
        })
    }
    pub(crate) fn decrypt_stored(&self, item: &StoredConnection) -> Result<SecretBundle> {
        self.inner.key.decrypt(&item.encrypted_secrets)
    }

    pub(crate) fn save_connection_in_document(
        &self,
        document: &mut VaultDocument,
        mut input: ConnectionInput,
        expected_updated_at: Option<&str>,
        private_key: Option<(&str, &str)>,
    ) -> Result<PublicConnection> {
            let existing_index = input.id.and_then(|id| {
                document
                    .connections
                    .iter()
                    .position(|connection| connection.id == id)
            });
            let existing = existing_index.map(|index| document.connections[index].clone());
            if input
                .id
                .is_some_and(|id| document.trash.iter().any(|item| item.connection.id == id))
            {
                bail!("This item is in the trash; restore it before editing");
            }
            if let Some(expected) = expected_updated_at {
                if existing
                    .as_ref()
                    .is_none_or(|item| item.updated_at != expected)
                {
                    bail!("This item changed or was deleted; close and reopen it before saving");
                }
            }
            let mut secrets = if let Some(connection) = &existing {
                self.inner
                    .key
                    .decrypt::<SecretBundle>(&connection.encrypted_secrets)?
            } else {
                SecretBundle::default()
            };
            anyhow::ensure!(input.secrets.oci.is_none(), "Legacy OCI configuration must be migrated before editing");
            let previous_password =
                zeroize::Zeroizing::new(secrets.get("password").map(str::to_owned));
            if let Some(old) = &existing {
                for module in &mut input.modules {
                    if module.id.is_nil() {
                        if let Some(previous) = old.modules.iter().find(|m| m.kind == module.kind && (m.kind != "customSecret" || m.name == module.name)) {
                            module.id = previous.id;
                        }
                    }
                }
            }
            {
                input.modules = normalize_modules(input.modules.clone())?;
                if let Some(connection) = &existing {
                    let retained = input
                        .modules
                        .iter()
                        .filter_map(ItemModule::secret_name)
                        .collect::<Vec<_>>();
                    for removed in connection
                        .modules
                        .iter()
                        .filter_map(ItemModule::secret_name)
                        .filter(|name| !retained.contains(name))
                    {
                        input.remove_secret_names.push(removed.to_owned());
                    }
                }
            }
            if let Some(old) = &existing {
                for module in &input.modules {
                    if let Some(previous) = old.modules.iter().find(|previous| previous.id == module.id && previous.name != module.name && previous.kind == module.kind) {
                        if let (Some(old_name), Some(new_name)) = (previous.secret_name(), module.secret_name()) {
                            if input.secrets.get(new_name).is_none() {
                                if let Some(value) = secrets.get(old_name) { input.secrets.named_secrets.insert(new_name.to_owned(), value.to_owned()); }
                            }
                        }
                    }
                }
            }
            merge_secret_bundle(&mut secrets, &input.secrets);
            for name in &input.remove_secret_names {
                secrets.named_secrets.remove(name.trim());
                match name.trim() {
                    "password" => secrets.password = None,
                    "passphrase" => secrets.passphrase = None,
                    "privateKey" | "private_key" => {
                        secrets.private_key = None;
                        secrets.private_key_name = None;
                    }
                    "token" => secrets.token = None,
                    "apiKey" | "api_key" => secrets.api_key = None,
                    "apiCredential" | "api_credential" => {
                        secrets.named_secrets.remove("apiCredential");
                        secrets.token = None;
                        secrets.api_key = None;
                    }
                    _ => {}
                }
            }

            input.name = checked_text(&input.name, "Project name", 80)?;
            if input.name.is_empty() {
                bail!("Project name cannot be empty");
            }
            if document
                .connections
                .iter()
                .enumerate()
                .any(|(index, item)| {
                    Some(index) != existing_index && same_item_name(&item.name, &input.name)
                })
            {
                bail!("Project name already exists; use a different name");
            }
            if input.modules.is_empty() && !matches!(input.http_auth_type.as_str(), "oci-signature" | "aws-sigv4") {
                bail!("Project must have at least one module");
            }

            let mut imported_key_name = existing
                .as_ref()
                .map(|v| v.private_key_name.clone())
                .unwrap_or_default();
            if let Some((key, name)) = private_key {
                if !input
                    .modules
                    .iter()
                    .any(|module| module.kind == "privateKey")
                {
                    bail!("Add an SSH private key module before saving a key");
                }
                secrets.private_key = Some(key.to_owned());
                secrets.private_key_name = Some(name.to_owned());
                imported_key_name = name.to_owned();
            } else if !input.private_key_import_path.trim().is_empty() {
                let path = PathBuf::from(input.private_key_import_path.trim());
                secrets.private_key = Some(
                    fs::read_to_string(&path).context("SSH private key must be in text format")?,
                );
                imported_key_name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("SSH KEY")
                    .to_owned();
                secrets.private_key_name = Some(imported_key_name.clone());
            }

            prepare_auto_api(&mut input, &mut secrets)?;
            secrets.remember_password_change(previous_password.as_deref());

            let now = Utc::now().to_rfc3339();
            let mut stored =
                normalize_connection(input, existing.as_ref(), &secrets, imported_key_name, now)?;
            stored.encrypted_secrets = self.inner.key.encrypt(&secrets)?;
            self.validate_ssh_jump(document, &stored, &secrets)?;
            let public = stored.public(Some(&secrets));
            if let Some(index) = existing_index {
                document.connections[index] = stored;
            } else {
                document.connections.push(stored);
            }
            Ok(public)
    }

    pub fn reorder_connections(&self, ids: &[Uuid], expected_order: &[Uuid]) -> Result<()> {
        self.update_document(|document| {
            let current: Vec<Uuid> = document.connections.iter().map(|item| item.id).collect();
            if current != expected_order {
                bail!("The item list changed; refresh before reordering");
            }
            let positions: std::collections::HashMap<Uuid, usize> = ids
                .iter()
                .copied()
                .enumerate()
                .map(|(index, id)| (id, index))
                .collect();
            if positions.len() != ids.len()
                || ids.len() != current.len()
                || current.iter().any(|id| !positions.contains_key(id))
            {
                bail!("The order must contain every current item exactly once");
            }
            // Order is presentation metadata; preserve revisions and encrypted payloads.
            document.connections.sort_by_key(|item| positions[&item.id]);
            Ok(())
        })
    }

    pub fn delete_connection(&self, id: Uuid) -> Result<()> {
        self.trash_connection(id, None)
    }

    pub fn trash_connection(&self, id: Uuid, expected_updated_at: Option<&str>) -> Result<()> {
        self.update_document(|document| {
            let index = document
                .connections
                .iter()
                .position(|item| item.id == id)
                .context("Connection not found")?;
            if expected_updated_at
                .is_some_and(|expected| document.connections[index].updated_at != expected)
            {
                bail!("This item changed; refresh before moving it to trash");
            }
            let connection = document.connections.remove(index);
            document.trash.insert(
                0,
                crate::model::StoredTrashItem {
                    deleted_at: Utc::now().to_rfc3339(),
                    connection,
                },
            );
            Ok(())
        })
    }

    pub fn trash_items(&self) -> Result<Vec<crate::model::PublicTrashItem>> {
        self.read_document()?
            .trash
            .into_iter()
            .map(|entry| {
                let secrets: SecretBundle = self
                    .inner
                    .key
                    .decrypt(&entry.connection.encrypted_secrets)?;
                Ok(crate::model::PublicTrashItem {
                    deleted_at: entry.deleted_at,
                    item: entry.connection.public(Some(&secrets)),
                })
            })
            .collect()
    }

    pub fn restore_connection(&self, id: Uuid, deleted_at: &str) -> Result<()> {
        self.update_document(|document| {
            let index = document
                .trash
                .iter()
                .position(|entry| entry.connection.id == id && entry.deleted_at == deleted_at)
                .context("Trash item changed; refresh before restoring")?;
            let mut stored = document.trash.remove(index).connection;
            stored.name = unique_import_name(&document.connections, &stored.name);
            if document.connections.iter().any(|item| item.id == stored.id) {
                stored.id = Uuid::new_v4();
            }
            stored.updated_at = Utc::now().to_rfc3339();
            document.connections.push(stored);
            Ok(())
        })
    }

    pub fn purge_connection(&self, id: Uuid, deleted_at: &str) -> Result<()> {
        self.update_document(|document| {
            let index = document
                .trash
                .iter()
                .position(|entry| entry.connection.id == id && entry.deleted_at == deleted_at)
                .context("Trash item changed; refresh before deleting permanently")?;
            document.trash.remove(index);
            Ok(())
        })
    }

    pub fn password_history(&self, id: Uuid) -> Result<Vec<crate::model::PasswordHistoryEntry>> {
        Ok(self.get_connection(id)?.secrets.password_history.clone())
    }

    pub fn clear_password_history(&self, id: Uuid, expected_updated_at: &str) -> Result<()> {
        self.update_document(|document| {
            let stored = document
                .connections
                .iter_mut()
                .find(|item| item.id == id)
                .context("Connection not found")?;
            if stored.updated_at != expected_updated_at {
                bail!("This item changed; refresh before clearing history");
            }
            let mut secrets: SecretBundle = self.inner.key.decrypt(&stored.encrypted_secrets)?;
            secrets.password_history.clear();
            stored.encrypted_secrets = self.inner.key.encrypt(&secrets)?;
            stored.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    pub fn set_connection_enabled(&self, id: Uuid, enabled: bool) -> Result<()> {
        self.update_document(|document| {
            let connection = document
                .connections
                .iter_mut()
                .find(|connection| connection.id == id)
                .context("Connection not found")?;
            connection.enabled = enabled;
            connection.updated_at = Utc::now().to_rfc3339();
            Ok(())
        })
    }

    pub fn activities(&self) -> Result<Vec<Activity>> {
        Ok(self.read_document()?.activities)
    }

    pub fn add_activity(&self, activity: NewActivity) -> Result<()> {
        self.update_document(|document| {
            document.activities.insert(
                0,
                Activity {
                    id: Uuid::new_v4(),
                    time: Utc::now().to_rfc3339(),
                    status: if activity.status == "error" {
                        "error"
                    } else {
                        "success"
                    }
                    .to_owned(),
                    source: clean_text(&activity.source, 40),
                    connection_name: clean_text(&activity.connection_name, 80),
                    action: clean_text(&activity.action, 160),
                    duration_ms: activity.duration_ms,
                    error: clean_text(&activity.error, 240),
                },
            );
            document.activities.truncate(MAX_ACTIVITIES);
            Ok(())
        })
    }

    pub fn clear_activities(&self) -> Result<()> {
        self.update_document(|document| {
            document.activities.clear();
            Ok(())
        })
    }

    pub fn app_state(
        &self,
        executable: &str,
        browser_bridge: crate::model::BrowserBridgeState,
        unlocked: bool,
    ) -> Result<AppState> {
        let document = self.read_document()?;
        let connections = if unlocked {
            document
                .connections
                .iter()
                .map(|connection| {
                    let secrets: SecretBundle =
                        self.inner.key.decrypt(&connection.encrypted_secrets)?;
                    Ok(connection.public(Some(&secrets)))
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        Ok(AppState {
            connections,
            activities: if unlocked { document.activities } else { Vec::new() },
            mcp: McpState {
                status: "ready".to_owned(),
                error: String::new(),
                endpoint: "stdio".to_owned(),
                stdio_command: executable.to_owned(),
            },
            browser_bridge,
            security: SecurityState {
                encrypted: true,
                storage: self.storage_label().to_owned(),
            },
            settings: document.settings.clone(),
        })
    }

    pub fn export_connections(&self) -> Result<Vec<(PortableConnection, SecretBundle)>> {
        Ok(self.export_backup_entries()?.0)
    }

    pub fn export_backup_entries(
        &self,
    ) -> Result<(
        Vec<(PortableConnection, SecretBundle)>,
        Vec<crate::model::PortableTrashItem>,
    )> {
        let document = self.read_document()?;
        let active = document
            .connections
            .into_iter()
            .map(|connection| {
                let secrets = self
                    .inner
                    .key
                    .decrypt::<SecretBundle>(&connection.encrypted_secrets)?;
                Ok((connection.portable(), secrets))
            })
            .collect::<Result<Vec<_>>>()?;
        let trash = document
            .trash
            .into_iter()
            .map(|entry| {
                let secrets = self
                    .inner
                    .key
                    .decrypt::<SecretBundle>(&entry.connection.encrypted_secrets)?;
                Ok(crate::model::PortableTrashItem {
                    deleted_at: entry.deleted_at,
                    connection: entry.connection.portable(),
                    secrets,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((active, trash))
    }

    pub fn merge_connections(
        &self,
        imported: Vec<(PortableConnection, SecretBundle)>,
    ) -> Result<ImportSummary> {
        self.merge_backup_entries(imported, Vec::new())
    }

    pub fn merge_backup_entries(
        &self,
        imported: Vec<(PortableConnection, SecretBundle)>,
        trash: Vec<crate::model::PortableTrashItem>,
    ) -> Result<ImportSummary> {
        self.update_document_with_result(|document| {
            let mut summary = ImportSummary {
                added: 0,
                merged: 0,
            };
            let mut ids = std::collections::HashSet::new();
            anyhow::ensure!(imported.iter().all(|(item, _)| ids.insert(item.id)) && trash.iter().all(|entry| ids.insert(entry.connection.id)), "Duplicate project IDs in imported snapshot");
            let mut imported = imported;
            imported.sort_by_key(|(item, _)| item.ssh_jump_item_id.is_some());
            let mut id_map = std::collections::HashMap::new();
            for (mut public, secrets) in imported {
                let source_id = public.id;
                if let Some(jump_id) = public.ssh_jump_item_id {
                    if !public.ssh_jump_unresolved {
                        if let Some(mapped) = id_map.get(&jump_id) {
                            public.ssh_jump_item_id = Some(*mapped);
                        } else {
                            public.ssh_jump_unresolved = true;
                        }
                    }
                }
                let mut stored = stored_from_portable(&public, &secrets, &self.inner.key)?;
                let imported_secrets: SecretBundle =
                    self.inner.key.decrypt(&stored.encrypted_secrets)?;
                let requested_name = clean_text(&stored.name, 80);
                stored.name = if requested_name.is_empty() {
                    "Import project".to_owned()
                } else {
                    requested_name
                };

                let has_requested_name = document
                    .connections
                    .iter()
                    .any(|item| same_item_name(&item.name, &stored.name));
                let mut duplicate = None;
                for existing in document.connections.iter().filter(|item| {
                    same_item_name(&item.name, &stored.name)
                        || (has_requested_name && import_name_in_family(&item.name, &stored.name))
                }) {
                    let existing_secrets: SecretBundle =
                        self.inner.key.decrypt(&existing.encrypted_secrets)?;
                    if same_portable_content(
                        existing,
                        &existing_secrets,
                        &stored,
                        &imported_secrets,
                    ) {
                        duplicate = Some(existing.id);
                        break;
                    }
                }

                if let Some(existing_id) = duplicate {
                    id_map.insert(source_id, existing_id);
                    summary.merged += 1;
                    continue;
                }

                stored.name = unique_import_name(&document.connections, &stored.name);
                if document.connections.iter().any(|item| item.id == stored.id)
                    || document
                        .trash
                        .iter()
                        .any(|item| item.connection.id == stored.id)
                {
                    stored.id = Uuid::new_v4();
                }
                id_map.insert(source_id, stored.id);
                document.connections.push(stored);
                summary.added += 1;
            }
            for entry in trash {
                chrono::DateTime::parse_from_rfc3339(&entry.deleted_at)
                    .context("Invalid trash deletion date")?;
                let mut stored =
                    stored_from_portable(&entry.connection, &entry.secrets, &self.inner.key)?;
                if let Some(id) = stored.ssh_jump_item_id {
                    if let Some(mapped) = id_map.get(&id) { stored.ssh_jump_item_id = Some(*mapped); }
                    else { stored.ssh_jump_unresolved = true; }
                }
                let imported_secrets: SecretBundle = self.inner.key.decrypt(&stored.encrypted_secrets)?;
                let mut duplicate = false;
                for existing in document.trash.iter().map(|entry| &entry.connection) {
                    if same_item_name(&existing.name, &stored.name) {
                        let secrets: SecretBundle =
                            self.inner.key.decrypt(&existing.encrypted_secrets)?;
                        if same_portable_content(existing, &secrets, &stored, &imported_secrets) {
                            duplicate = true;
                            break;
                        }
                    }
                }
                if duplicate {
                    summary.merged += 1;
                    continue;
                }
                if document.connections.iter().any(|item| item.id == stored.id)
                    || document
                        .trash
                        .iter()
                        .any(|item| item.connection.id == stored.id)
                {
                    stored.id = Uuid::new_v4();
                }
                document.trash.push(crate::model::StoredTrashItem {
                    deleted_at: entry.deleted_at,
                    connection: stored,
                });
                summary.added += 1;
            }
            document.activities.insert(
                0,
                Activity {
                    id: Uuid::new_v4(),
                    time: Utc::now().to_rfc3339(),
                    status: "success".to_owned(),
                    source: "Apply".to_owned(),
                    connection_name: "Portable backup".to_owned(),
                    action: format!(
                        "Import backup: added {}, merged duplicates {}",
                        summary.added, summary.merged
                    ),
                    duration_ms: 0,
                    error: String::new(),
                },
            );
            document.activities.truncate(MAX_ACTIVITIES);
            Ok(summary)
        })
    }

    fn read_document(&self) -> Result<VaultDocument> {
        let lock = self.open_lock()?;
        FileExt::lock_shared(&lock).context("Could not lock vault")?;
        let result = self.read_document_unlocked();
        let _ = FileExt::unlock(&lock);
        result
    }

    fn read_document_unlocked(&self) -> Result<VaultDocument> {
        let bytes = fs::read(&self.inner.vault_path).context("Could not read vault")?;
        let mut document: VaultDocument =
            serde_json::from_slice(&bytes).context("Invalid vault file format")?;
        if !matches!(document.version, 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | VAULT_VERSION) {
            bail!("Unsupported vault version {}", document.version);
        }
        document.version = VAULT_VERSION;
        for item in document.connections.iter_mut().chain(document.trash.iter_mut().map(|entry| &mut entry.connection)) {
            item.revision = item.revision.max(1);
            for (index, module) in item.modules.iter_mut().enumerate() {
                if module.id.is_nil() {
                    use sha2::{Digest, Sha256};
                    let digest = Sha256::digest(format!("{}:module:{index}", item.id).as_bytes());
                    let mut bytes = [0u8; 16]; bytes.copy_from_slice(&digest[..16]);
                    module.id = Uuid::from_bytes(bytes);
                }
            }
        }
        Ok(document)
    }

    fn write_document_unlocked(&self, document: &VaultDocument) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(document).context("Could not serialize vault")?;
        let mut file = AtomicWriteFile::open(&self.inner.vault_path)
            .context("Could not create temporary vault file")?;
        file.write_all(&bytes).context("Could not write vault")?;
        file.commit().context("Could not commit vault file")?;
        Ok(())
    }

    fn update_document<F>(&self, update: F) -> Result<()>
    where
        F: FnOnce(&mut VaultDocument) -> Result<()>,
    {
        self.with_exclusive_lock(|_| {
            let mut document = self.read_document_unlocked()?;
            let before = document.clone();
            update(&mut document)?;
            advance_revisions(&before, &mut document);
            self.write_document_unlocked(&document)
        })
    }

    pub(crate) fn update_document_with_result<T, F>(&self, update: F) -> Result<T>
    where
        F: FnOnce(&mut VaultDocument) -> Result<T>,
    {
        self.with_exclusive_lock(|_| {
            let mut document = self.read_document_unlocked()?;
            let before = document.clone();
            let result = update(&mut document)?;
            advance_revisions(&before, &mut document);
            self.write_document_unlocked(&document)?;
            Ok(result)
        })
    }

    fn with_exclusive_lock<T, F>(&self, action: F) -> Result<T>
    where
        F: FnOnce(&File) -> Result<T>,
    {
        let lock = self.open_lock()?;
        lock.lock_exclusive().context("Could not lock vault")?;
        let result = action(&lock);
        let _ = FileExt::unlock(&lock);
        result
    }

    fn open_lock(&self) -> Result<File> {
        OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&self.inner.lock_path)
            .context("Could not open vault lock file")
    }
}

fn migrate_legacy_oci_input(input: &mut ConnectionInput) -> Result<()> {
    let Some(credential) = input.secrets.oci.take() else {
        return Ok(());
    };
    anyhow::ensure!(
        input.oci_bindings.is_none(),
        "Legacy OCI configuration conflicts with module bindings"
    );
    for module in &mut input.modules {
        if module.id.is_nil() {
            module.id = Uuid::new_v4();
        }
    }
    let mut add = |name: &str, value: &str, visible: bool| {
        let mut unique = name.to_owned();
        let mut suffix = 2;
        while input.modules.iter().any(|module| {
            module
                .secret_name()
                .is_some_and(|name| normalized_field_name(name) == normalized_field_name(&unique))
        }) || input
            .secrets
            .named_secrets
            .keys()
            .any(|name| normalized_field_name(name) == normalized_field_name(&unique))
        {
            unique = format!("{name}_{suffix}");
            suffix += 1;
        }
        let id = Uuid::new_v4();
        input.modules.push(ItemModule {
            id,
            kind: "customSecret".into(),
            name: unique.clone(),
            value: String::new(),
            agent_visible: Some(visible),
            alternate_values: Vec::new(),
        });
        if !value.is_empty() {
            input.secrets.named_secrets.insert(unique, value.to_owned());
        }
        id
    };
    let region = add("region", &credential.config.region, true);
    let tenancy = add("tenancy_ocid", &credential.config.tenancy_ocid, true);
    let user = add("user_ocid", &credential.config.user_ocid, false);
    let fingerprint = add("fingerprint", &credential.config.fingerprint, false);
    let signing_key = add(
        "oci_signing_private_key",
        credential.key_pem.as_deref().unwrap_or_default(),
        false,
    );
    if !input
        .modules
        .iter()
        .any(|module| module.kind == "privateKey")
    {
        let module = input
            .modules
            .iter_mut()
            .find(|module| module.id == signing_key)
            .unwrap();
        let value = input.secrets.named_secrets.get(&module.name).cloned();
        input.secrets.named_secrets.remove(&module.name);
        module.kind = "privateKey".to_owned();
        module.name = "privateKey".to_owned();
        input.secrets.private_key = value;
    }
    let url = if let Some(module) = input.modules.iter().find(|module| module.kind == "url") {
        module.id
    } else {
        let id = Uuid::new_v4();
        input.modules.push(ItemModule {
            id,
            kind: "url".into(),
            name: String::new(),
            value: credential.config.base_url()?.to_string(),
            agent_visible: Some(true),
            alternate_values: Vec::new(),
        });
        id
    };
    input.oci_bindings = Some(crate::model::OciModuleBindings {
        region: Some(region),
        service_url: Some(url),
        tenancy_ocid: Some(tenancy),
        user_ocid: Some(user),
        fingerprint: Some(fingerprint),
        private_key: Some(signing_key),
    });
    Ok(())
}

fn same_certificate_authority(left: &StoredConnection, right: &StoredConnection) -> bool {
    let binding = |item: &StoredConnection| item.certificate_authority.as_ref().map(|b| {
        let position = |id| item.modules.iter().position(|m| m.id == id).map_or_else(|| format!("missing:{id}"), |n| format!("module:{n}"));
        (b.kind, position(b.private_key), b.ca_certificate.map(position))
    });
    binding(left) == binding(right)
}

fn same_aws_bindings(left: &StoredConnection, right: &StoredConnection) -> bool {
    let positions = |item: &StoredConnection| {
        item.aws_bindings.as_ref().map(|binding| {
            [binding.region, binding.service_url, binding.access_key_id, binding.secret_access_key, binding.session_token]
                .map(|id| id.map(|id| item.modules.iter().position(|module| module.id == id).map_or_else(|| format!("missing:{id}"), |position| format!("module:{position}"))))
        })
    };
    positions(left) == positions(right)
}

fn same_oci_bindings(left: &StoredConnection, right: &StoredConnection) -> bool {
    let positions = |item: &StoredConnection| {
        item.oci_bindings.as_ref().map(|binding| {
            [
                binding.region,
                binding.service_url,
                binding.tenancy_ocid,
                binding.user_ocid,
                binding.fingerprint,
                binding.private_key,
            ]
            .map(|id| id.and_then(|id| item.modules.iter().position(|module| module.id == id)))
        })
    };
    positions(left) == positions(right)
}
fn canonical_secret_module_name(kind: &str) -> Option<&str> {
    match kind {
        "username" => Some("username"),
        "password" => Some("password"),
        "apiCredential" => Some("apiCredential"),
        "privateKey" => Some("privateKey"),
        "passphrase" => Some("passphrase"),
        "totp" => Some("totp"),
        kind if crate::item_types::personal_field(kind) => Some(kind),
        _ => None,
    }
}

fn normalize_modules(modules: Vec<ItemModule>) -> Result<Vec<ItemModule>> {
    let mut output = Vec::new();
    let mut ids = std::collections::HashSet::new();
    let mut singleton_kinds = Vec::new();
    let mut secret_names: Vec<String> = Vec::new();
    for mut module in modules {
        if module.id.is_nil() { module.id = Uuid::new_v4(); }
        if !ids.insert(module.id) { bail!("Duplicate module ID"); }
        module.kind = module.kind.trim().to_owned();
        if !crate::item_types::personal_field(&module.kind)
            && !matches!(
                module.kind.as_str(),
                "username"
                    | "password"
                    | "apiCredential"
                    | "privateKey"
                    | "passphrase"
                    | "totp"
                    | "customSecret"
                    | "host"
                    | "port"
                    | "url"
                    | "publicKey" | "sshCertificate" | "certificate" | "csr" | "caCertificate"
            )
        {
            bail!("Unsupported module:{}", module.kind);
        }
        if !module.alternate_values.is_empty() && !matches!(module.kind.as_str(), "host" | "url") {
            bail!("Only host and url modules support other addresses");
        }
        if module.agent_visible.is_none() {
            module.agent_visible = Some(!module_kind_has_plaintext_reveal(&module.kind));
        }
        if module.kind != "customSecret" {
            if singleton_kinds.contains(&module.kind) {
                bail!("Duplicate module:{}", module.kind);
            }
            singleton_kinds.push(module.kind.clone());
        }
        if let Some(name) = canonical_secret_module_name(&module.kind) {
            module.name = name.to_owned();
            module.value.clear();
        } else if module.kind == "customSecret" {
            module.name = checked_text(&module.name, "Custom secret field name", 80)?;
            if !valid_custom_field_name(&module.name) || reserved_secret_name(&module.name) {
                bail!("Invalid custom secret field name:{}", module.name);
            }
            module.value.clear();
        } else {
            module.name.clear();
            module.value = module.value.trim().to_owned();
            if module.kind == "port" && !module.value.is_empty() {
                let port = module
                    .value
                    .parse::<u16>()
                    .context("Port must be 1–65535")?;
                if port == 0 {
                    bail!("Port must be 1–65535");
                }
                module.value = port.to_string();
            }
        }
        if matches!(module.kind.as_str(), "host" | "url") {
            if !module.alternate_values.is_empty() && module.value.is_empty() {
                bail!("A default address is required when other addresses are saved");
            }
            let mut addresses = std::collections::HashSet::new();
            let has_alternates = !module.alternate_values.is_empty();
            for address in std::iter::once(&mut module.value).chain(module.alternate_values.iter_mut()) {
                if address.trim().is_empty() {
                    if !has_alternates { continue; }
                    bail!("Address cannot be empty");
                }
                *address = normalize_module_address(&module.kind, address)?;
                if !addresses.insert(address.clone()) { bail!("Duplicate address"); }
            }
        }
        if let Some(name) = module.secret_name() {
            if secret_names
                .iter()
                .any(|candidate| normalized_field_name(candidate) == normalized_field_name(name))
            {
                bail!("Duplicate secret module field name:{name}");
            }
            secret_names.push(name.to_owned());
        }
        output.push(module);
    }
    Ok(output)
}

fn normalize_module_address(kind: &str, address: &str) -> Result<String> {
    if kind == "host" { return ssh_endpoint_host(address); }
    let normalized = api_catalog::normalize_base_url(address, "")?;
    let parsed = Url::parse(&normalized).context("Invalid API URL")?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none()
        || !parsed.username().is_empty() || parsed.password().is_some() {
        bail!("API URL must use HTTP/HTTPS and must not include an account or password in the address");
    }
    Ok(normalized)
}

fn module_value<'a>(modules: &'a [ItemModule], kind: &str) -> Option<&'a str> {
    modules
        .iter()
        .find(|module| module.kind == kind)
        .map(|module| module.value.trim())
        .filter(|value| !value.is_empty())
}

fn module_secret_configured(modules: &[ItemModule], secrets: &SecretBundle, kind: &str) -> bool {
    modules
        .iter()
        .find(|module| module.kind == kind)
        .and_then(ItemModule::secret_name)
        .and_then(|name| secrets.get(name))
        .is_some()
}

fn module_fields(modules: &[ItemModule]) -> Vec<SecretField> {
    modules
        .iter()
        .filter_map(|module| {
            module.secret_name().map(|name| SecretField {
                name: name.to_owned(),
                kind: if module.kind == "totp" {
                    "totp"
                } else {
                    "text"
                }
                .to_owned(),
            })
        })
        .collect()
}

fn push_module(modules: &mut Vec<ItemModule>, kind: &str, name: &str, value: &str) {
    if kind != "customSecret" && modules.iter().any(|module| module.kind == kind) {
        return;
    }
    if kind == "customSecret" && modules.iter().any(|module| module.name == name) {
        return;
    }
    modules.push(ItemModule {
        alternate_values: Vec::new(),
        id: uuid::Uuid::nil(),
        kind: kind.to_owned(),
        name: name.to_owned(),
        value: value.to_owned(),
        agent_visible: Some(!module_kind_has_plaintext_reveal(kind)),
    });
}

fn fallback_item_name(
    requested: &str,
    id: Uuid,
    modules: &[ItemModule],
    secrets: &SecretBundle,
) -> String {
    let requested = clean_text(requested, 80);
    if !requested.is_empty() {
        return requested;
    }
    if let Some(host) = module_value(modules, "host") {
        return clean_text(host, 80);
    }
    if let Some(url) = module_value(modules, "url") {
        let profile = api_catalog::infer("", url, secrets.get("apiCredential").unwrap_or_default());
        return api_catalog::fallback_name(url, profile);
    }
    if let Some(secret) = secrets.get("apiCredential") {
        let inferred = api_catalog::fallback_name("", api_catalog::infer("", "", secret));
        if inferred != "API" {
            return inferred;
        }
    }
    let short = id.simple().to_string()[..6].to_ascii_uppercase();
    if module_secret_configured(modules, secrets, "username")
        && module_secret_configured(modules, secrets, "password")
    {
        format!("LOGIN-{short}")
    } else {
        format!("ITEM-{short}")
    }
}

pub(crate) fn normalize_connection(
    input: ConnectionInput,
    existing: Option<&StoredConnection>,
    secrets: &SecretBundle,
    private_key_name: String,
    now: String,
) -> Result<StoredConnection> {
    crate::login_uri::validate(&input.login_uris)?;
    let modules = normalize_modules(input.modules)?;
    let mut oci_bindings = input.oci_bindings;
    if let Some(bindings) = &mut oci_bindings {
        for binding in [
            &mut bindings.region,
            &mut bindings.service_url,
            &mut bindings.tenancy_ocid,
            &mut bindings.user_ocid,
            &mut bindings.fingerprint,
            &mut bindings.private_key,
        ] {
            if let Some(id) = *binding {
                if !modules.iter().any(|module| module.id == id) {
                    anyhow::ensure!(
                        existing
                            .is_some_and(|old| old.modules.iter().any(|module| module.id == id)),
                        "OCI binding must reference a module in this project"
                    );
                    *binding = None;
                }
            }
        }
    }
    let mut aws_bindings = input.aws_bindings;
    if let Some(bindings) = &mut aws_bindings {
        for binding in [
            &mut bindings.region,
            &mut bindings.service_url,
            &mut bindings.access_key_id,
            &mut bindings.secret_access_key,
            &mut bindings.session_token,
        ] {
            if let Some(id) = *binding {
                if !modules.iter().any(|module| module.id == id) {
                    anyhow::ensure!(
                        existing.is_some_and(|old| old.modules.iter().any(|module| module.id == id)),
                        "AWS binding must reference a module in this project"
                    );
                    *binding = None;
                }
            }
        }
    }
    if input.http_auth_type == "aws-sigv4" {
        crate::aws::validate_modules(&modules, secrets, aws_bindings.as_ref())?;
    }
    if input.http_auth_type == crate::oci::AUTH_MODE {
        crate::oci::validate_modules(&modules, secrets, oci_bindings.as_ref())?;
    }
    let item_type = crate::item_types::normalize(&input.item_type, &modules)?;

    let id = existing
        .map(|value| value.id)
        .or(input.id)
        .unwrap_or_else(Uuid::new_v4);
    let created_at = existing
        .map(|value| value.created_at.clone())
        .unwrap_or_else(|| now.clone());
    let host = module_value(&modules, "host")
        .unwrap_or_default()
        .to_owned();
    let port = module_value(&modules, "port")
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    let service_url = if input.http_auth_type == crate::oci::AUTH_MODE {
        oci_bindings
            .as_ref()
            .and_then(|bindings| crate::oci::binding_value(&modules, secrets, bindings.service_url))
    } else if input.http_auth_type == "aws-sigv4" {
        aws_bindings.as_ref().and_then(|bindings| crate::oci::binding_value(&modules, secrets, bindings.service_url))
    } else {
        module_value(&modules, "url")
    };
    let base_url = if let Some(raw) = service_url {
        let normalized = api_catalog::normalize_base_url(raw, "")?;
        let parsed = Url::parse(&normalized).context("Invalid API URL")?;
        if !matches!(parsed.scheme(), "http" | "https")
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            bail!(
                "API URL must use HTTP/HTTPS and must not include an account or password in the address"
            );
        }
        normalized
    } else {
        String::new()
    };
    let inferred_ssh_auth_type = if module_secret_configured(&modules, secrets, "password") {
        "password"
    } else if module_secret_configured(&modules, secrets, "privateKey") {
        "privateKey"
    } else {
        ""
    }
    .to_owned();
    let requested_ssh_auth_type = if input.ssh_auth_type.is_empty() {
        existing
            .map(|old| old.ssh_auth_type.as_str())
            .unwrap_or_default()
    } else {
        if !matches!(input.ssh_auth_type.as_str(), "password" | "privateKey" | "certificate") {
            bail!("Invalid SSH authentication mode");
        }
        &input.ssh_auth_type
    };
    // Incomplete certificate configuration must not fall back to password authentication.
    let ssh_auth_type = if requested_ssh_auth_type == "certificate" || module_secret_configured(&modules, secrets, requested_ssh_auth_type) {
        requested_ssh_auth_type.to_owned()
    } else {
        inferred_ssh_auth_type
    };
    let http_auth_type = input.http_auth_type.clone();
    let name = fallback_item_name(&input.name, id, &modules, secrets);
    Ok(StoredConnection {
        certificate_authority: existing.and_then(|old| old.certificate_authority.clone()).or(input.certificate_authority),
        oci_bindings,
        aws_bindings,
        ssh_jump_item_id: input.ssh_jump_item_id,
        ssh_jump_unresolved: input.ssh_jump_unresolved,
        revision: existing.map_or(1, |old| old.revision.saturating_add(1)),
        login_uris: input.login_uris,
        item_type,
        folder: checked_text(&input.folder, "Folder", 120)?,
        favorite: input.favorite,
        id,
        modules: modules.clone(),
        name,
        enabled: input.enabled,
        created_at,
        updated_at: now,
        description: checked_text(&input.description, "Project description", 240)?,
        host,
        port,
        ssh_auth_type,
        http_auth_type,
        private_key_name: if modules.iter().any(|module| module.kind == "privateKey") {
            private_key_name
        } else {
            String::new()
        },
        base_url,
        auth_header: input.auth_header.trim().to_owned(),
        auth_location: input.auth_location.trim().to_owned(),
        auth_prefix: input.auth_prefix.trim().to_owned(),
        api_auth_headers: input.api_auth_headers,
        test_path: input.test_path.trim().to_owned(),
        secret: Some(SecretProfile {
            fields: module_fields(&modules),
        }),
        encrypted_secrets: empty_envelope(),
    })
}

fn stored_from_portable(
    portable: &PortableConnection,
    secrets: &SecretBundle,
    key: &MasterKey,
) -> Result<StoredConnection> {
    let mut input = ConnectionInput {
        certificate_authority: portable.certificate_authority.clone(),
        oci_bindings: portable.oci_bindings.clone(),
        aws_bindings: portable.aws_bindings.clone(),
        ssh_jump_item_id: portable.ssh_jump_item_id,
        ssh_jump_unresolved: portable.ssh_jump_unresolved,
        ssh_auth_type: portable.ssh_auth_type.clone(),
        login_uris: portable.login_uris.clone(),
        item_type: portable.item_type.clone(),
        folder: portable.folder.clone(),
        favorite: portable.favorite,
        id: Some(portable.id),
        modules: portable.modules.clone(),
        name: portable.name.clone(),
        enabled: portable.enabled,
        description: portable.description.clone(),
        http_auth_type: portable.http_auth_type.clone(),
        private_key_import_path: String::new(),
        auth_header: portable.auth_header.clone(),
        auth_location: portable.auth_location.clone(),
        auth_prefix: portable.auth_prefix.clone(),
        api_auth_headers: portable.api_auth_headers.clone(),
        test_path: portable.test_path.clone(),
        remove_secret_names: Vec::new(),
        secrets: secrets.clone(),
    };
    migrate_legacy_oci_input(&mut input)?;
    let secrets = input.secrets.clone();
    let mut stored = normalize_connection(
        input,
        None,
        &secrets,
        portable.private_key_name.clone(),
        portable.updated_at.clone(),
    )?;
    stored.created_at = portable.created_at.clone();
    stored.revision = portable.revision.max(1);
    stored.encrypted_secrets = key.encrypt(&secrets)?;
    Ok(stored)
}

fn same_portable_content(
    left: &StoredConnection,
    left_secrets: &SecretBundle,
    right: &StoredConnection,
    right_secrets: &SecretBundle,
) -> bool {
    if !same_certificate_authority(left, right)
        || !same_oci_bindings(left, right)
        || !same_aws_bindings(left, right)
        || left.ssh_jump_item_id != right.ssh_jump_item_id
        || left.ssh_jump_unresolved != right.ssh_jump_unresolved
        || left.login_uris != right.login_uris
        || !left.modules.iter().zip(&right.modules).all(|(a,b)| {
            let equal_value = |left: &str, right: &str| {
                left == right || matches!(a.kind.as_str(), "host" | "url")
                    && normalize_module_address(&a.kind, left).ok().zip(normalize_module_address(&a.kind, right).ok()).is_some_and(|(left, right)| left == right)
            };
            a.kind == b.kind && a.name == b.name && equal_value(&a.value, &b.value)
                && a.alternate_values.len() == b.alternate_values.len()
                && a.alternate_values.iter().zip(&b.alternate_values).all(|(a,b)| equal_value(a,b))
                && a.agent_visible == b.agent_visible
        })
        || left.modules.len() != right.modules.len()
        || left.item_type() != right.item_type()
        || left.folder != right.folder
        || left.favorite != right.favorite
        || left.description != right.description
        || left.enabled != right.enabled
        || left.ssh_auth_type != right.ssh_auth_type
        || left.http_auth_type != right.http_auth_type
        || left.auth_header != right.auth_header
        || left.auth_location != right.auth_location
        || left.auth_prefix != right.auth_prefix
        || left.api_auth_headers != right.api_auth_headers
        || left.test_path != right.test_path
        || left.private_key_name != right.private_key_name
    {
        return false;
    }
    if left_secrets.oci.as_ref().map(|oci| &oci.config)
        != right_secrets.oci.as_ref().map(|oci| &oci.config)
        || left_secrets.passkeys != right_secrets.passkeys
        || left_secrets.password_history != right_secrets.password_history
    {
        return false;
    }
    for module in &left.modules {
        let Some(name) = module.secret_name() else {
            continue;
        };
        if left_secrets.get(name) != right_secrets.get(name) {
            return false;
        }
    }
    true
}

fn import_name_in_family(candidate: &str, requested: &str) -> bool {
    let Some((candidate_base, candidate_index)) = split_numbered_import_name(candidate) else {
        return false;
    };
    let (requested_base, requested_index) = split_numbered_import_name(requested)
        .map(|(base, index)| (base, index + 1))
        .unwrap_or((requested, 2));
    same_item_name(candidate_base, requested_base) && candidate_index >= requested_index
}

fn split_numbered_import_name(name: &str) -> Option<(&str, usize)> {
    let without_closing = name.strip_suffix(')')?;
    let opening = without_closing.rfind('(')?;
    let index = without_closing[opening + 1..].parse::<usize>().ok()?;
    ((2..usize::MAX).contains(&index) && opening > 0)
        .then_some((&without_closing[..opening], index))
}

fn unique_import_name(connections: &[StoredConnection], requested: &str) -> String {
    unique_item_name(requested, |candidate| {
        connections
            .iter()
            .any(|item| same_item_name(&item.name, candidate))
    })
}

fn unique_item_name<F>(requested: &str, exists: F) -> String
where
    F: Fn(&str) -> bool,
{
    if !exists(requested) {
        return requested.to_owned();
    }
    let (base, mut index) = split_numbered_import_name(requested)
        .map(|(base, index)| (base, index + 1))
        .unwrap_or((requested, 2));
    loop {
        let candidate = format!("{base}({index})");
        if !exists(&candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn same_item_name(left: &str, right: &str) -> bool {
    left.trim().to_lowercase() == right.trim().to_lowercase()
}

fn empty_envelope() -> crate::model::SecretEnvelope {
    crate::model::SecretEnvelope {
        version: 0,
        nonce: String::new(),
        ciphertext: String::new(),
    }
}

fn merge_secret_bundle(current: &mut SecretBundle, next: &SecretBundle) {
    if next.private_key.as_ref().is_some_and(|value| !value.is_empty()) { current.private_key = next.private_key.clone(); }

    if next.password.as_ref().is_some_and(|v| !v.is_empty()) {
        current.password = next.password.clone();
    }
    if next.passphrase.as_ref().is_some_and(|v| !v.is_empty()) {
        current.passphrase = next.passphrase.clone();
    }
    if next.token.as_ref().is_some_and(|v| !v.is_empty()) {
        current.token = next.token.clone();
    }
    if next.api_key.as_ref().is_some_and(|v| !v.is_empty()) {
        current.api_key = next.api_key.clone();
    }
    for (name, value) in next.named_secrets.iter() {
        if !value.is_empty() {
            current.named_secrets.insert(name.clone(), value.clone());
        }
    }
}

fn prepare_auto_api(input: &mut ConnectionInput, secrets: &mut SecretBundle) -> Result<()> {
    if input.modules.iter().any(|module| !module.alternate_values.is_empty() && module.value.trim().is_empty()) {
        bail!("A default address is required when other addresses are saved");
    }
    if input.http_auth_type != "auto" {
        return Ok(());
    }
    let has_api_credential = input
        .modules
        .iter()
        .any(|module| module.kind == "apiCredential");
    if !has_api_credential {
        let basic_ready = module_value(&input.modules, "url").is_some()
            && module_secret_configured(&input.modules, secrets, "username")
            && module_secret_configured(&input.modules, secrets, "password");
        if basic_ready {
            input.http_auth_type = "basic".to_owned();
            input.auth_header = "Authorization".to_owned();
            input.auth_location = "header".to_owned();
            input.auth_prefix.clear();
            input.api_auth_headers.clear();
        }
        return Ok(());
    }
    let Some(secret) = secrets
        .get("apiCredential")
        .filter(|value| !value.trim().is_empty())
        .map(str::to_owned)
    else {
        // An empty API credential module is a valid draft.  It becomes callable
        // only after a value is configured and capabilities are derived again.
        return Ok(());
    };
    let current_url = module_value(&input.modules, "url").unwrap_or_default();
    let profile = api_catalog::infer(&input.name, current_url, &secret);
    let normalized_url = api_catalog::normalize_base_url(current_url, profile.default_base_url)?;
    if !normalized_url.is_empty() {
        if let Some(module) = input.modules.iter_mut().find(|module| module.kind == "url") {
            module.value = normalized_url;
        } else {
            push_module(&mut input.modules, "url", "", &normalized_url);
        }
    }
    input.http_auth_type = profile.auth_type.to_owned();
    input.auth_header = profile.auth_header.to_owned();
    input.auth_location = profile.auth_location.to_owned();
    input.auth_prefix = profile.auth_prefix.to_owned();
    input.api_auth_headers.clear();
    input.test_path.clear();
    Ok(())
}

fn valid_custom_field_name(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(char::is_control)
}

fn normalized_field_name(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_whitespace() && !matches!(character, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn reserved_secret_name(value: &str) -> bool {
    crate::item_types::CARD_FIELDS
        .iter()
        .chain(crate::item_types::IDENTITY_FIELDS)
        .chain(std::iter::once(&"secureNote"))
        .any(|name| normalized_field_name(name) == normalized_field_name(value))
        || matches!(
            normalized_field_name(value).as_str(),
            "host"
                | "port"
                | "url"
                | "username"
                | "password"
                | "apicredential"
                | "privatekey"
                | "passphrase"
                | "totp"
                | "token"
                | "apikey"
        )
}

fn checked_text(value: &str, label: &str, max_chars: usize) -> Result<String> {
    let value = value.trim();
    if value.chars().count() > max_chars {
        bail!("{label}Cannot exceed {max_chars} characters");
    }
    Ok(value.to_owned())
}

fn clean_text(value: &str, max_chars: usize) -> String {
    value.trim().chars().take(max_chars).collect()
}

fn random_token() -> Result<String> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut token = [0_u8; 32];
    getrandom::fill(&mut token)
        .map_err(|error| anyhow::anyhow!("Could not generate MCP access token:{error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_metadata_survives_manual_edit_and_backup() {
        let dir = tempdir().unwrap();
        let vault = Vault::open(dir.path().join("v")).unwrap();
        let input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "metadata", "modules": [{"kind":"host", "value":"example.test"}]
        })).unwrap();
        let first = vault.save_connection(input.clone()).unwrap();
        let json = serde_json::to_value(&first).unwrap();
        assert_eq!(json["revision"], 1);
        assert!(json["modules"][0]["id"].as_str().is_some());
        let mut edit = input;
        edit.id = Some(first.id);
        edit.description = "manual edit".into();
        let second = vault.save_connection(edit).unwrap();
        let next = serde_json::to_value(&second).unwrap();
        assert_eq!(next["revision"], 2);
        assert_eq!(next["modules"][0]["id"], json["modules"][0]["id"]);
        let dest = Vault::open(dir.path().join("dest")).unwrap();
        dest.merge_connections(vault.export_connections().unwrap()).unwrap();
        let restored = serde_json::to_value(dest.list_connections().unwrap()).unwrap();
        assert_eq!(restored[0]["modules"][0]["id"], json["modules"][0]["id"]);
    }

    #[test]
    fn module_normalization_preserves_editor_order() {
        let modules = vec![
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "url".to_owned(),
                name: String::new(),
                value: "https://example.test".to_owned(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "password".to_owned(),
                name: String::new(),
                value: String::new(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "host".to_owned(),
                name: String::new(),
                value: "example.test".to_owned(),
                agent_visible: None,
            },
        ];

        let normalized = normalize_modules(modules).unwrap();
        assert_eq!(
            normalized
                .iter()
                .map(|module| module.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["url", "password", "host"]
        );
        assert_eq!(
            normalized
                .iter()
                .map(|module| module.agent_visible)
                .collect::<Vec<_>>(),
            vec![Some(true), Some(false), Some(true)]
        );
    }

    fn api_input(id: Option<Uuid>, name: &str, token: &str) -> ConnectionInput {
        let mut secrets = SecretBundle::default();
        secrets
            .named_secrets
            .insert("apiCredential".into(), token.to_owned());
        ConnectionInput {
            certificate_authority: None,
            aws_bindings: None,
            oci_bindings: None,
            ssh_jump_item_id: None,
            ssh_jump_unresolved: false,
            ssh_auth_type: String::new(),
            login_uris: Vec::new(),
            item_type: String::new(),
            folder: String::new(),
            favorite: false,
            id,
            modules: vec![
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "url".into(),
                    value: "https://api.example.test/v1/".into(),
                    ..Default::default()
                },
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "apiCredential".into(),
                    ..Default::default()
                },
            ],
            name: name.to_owned(),
            enabled: true,
            description: "test connection".to_owned(),
            http_auth_type: "bearer".to_owned(),
            private_key_import_path: String::new(),
            auth_header: "X-API-Key".to_owned(),
            auth_location: "header".to_owned(),
            auth_prefix: String::new(),
            api_auth_headers: vec![],
            test_path: "/health".to_owned(),
            remove_secret_names: vec![],
            secrets,
        }
    }

    fn ssh_input(id: Option<Uuid>) -> ConnectionInput {
        let mut secrets = SecretBundle::default();
        secrets.password = Some("test-password".to_owned());
        secrets
            .named_secrets
            .insert("username".into(), "root".into());
        ConnectionInput {
            certificate_authority: None,
            aws_bindings: None,
            oci_bindings: None,
            ssh_jump_item_id: None,
            ssh_jump_unresolved: false,
            ssh_auth_type: String::new(),
            login_uris: Vec::new(),
            item_type: String::new(),
            folder: String::new(),
            favorite: false,
            id,
            modules: vec![
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "host".into(),
                    value: "127.0.0.1".into(),
                    ..Default::default()
                },
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "port".into(),
                    value: "22".into(),
                    ..Default::default()
                },
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "username".into(),
                    ..Default::default()
                },
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    ..Default::default()
                },
            ],
            name: "ssh-test".to_owned(),
            enabled: true,
            description: String::new(),
            http_auth_type: String::new(),
            private_key_import_path: String::new(),
            auth_header: String::new(),
            auth_location: String::new(),
            auth_prefix: String::new(),
            api_auth_headers: vec![],
            test_path: String::new(),
            remove_secret_names: vec![],
            secrets,
        }
    }

    #[test]
    fn vault_file_contains_no_plain_secret_and_remains_valid_json() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        vault
            .save_connection(api_input(None, "plain-check", "unique-secret-marker-9437"))
            .unwrap();

        let contents = fs::read_to_string(vault_dir.join("vault.json")).unwrap();
        assert!(!contents.contains("unique-secret-marker-9437"));
        let stored_json: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(stored_json["version"], VAULT_VERSION);
        assert!(stored_json["connections"][0].get("type").is_none());
        assert!(stored_json["connections"][0].get("capabilities").is_none());
        assert!(serde_json::from_str::<VaultDocument>(&contents).is_ok());
        let public = vault.list_connections().unwrap();
        let serialized = serde_json::to_string(&public).unwrap();
        assert!(!serialized.contains("unique-secret-marker-9437"));
        let public_json = serde_json::to_value(&public).unwrap();
        assert!(public_json[0].get("type").is_none());
        assert_eq!(
            public_json[0]["capabilities"],
            serde_json::json!(["fill", "http"])
        );
    }

    #[test]
    fn v7_vault_upgrades_with_backup_without_changing_secrets() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        let saved = vault
            .save_connection(api_input(None, "Existing API", "existing-secret-marker"))
            .unwrap();
        let path = vault_dir.join("vault.json");
        let mut fixture: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        fixture["version"] = serde_json::json!(7);
        let encrypted = fixture["connections"][0]["encryptedSecrets"].clone();
        let obsolete = serde_json::json!({
            "type": "ssh",
            "capabilities": ["ssh"],
            "username": "",
            "authType": "obsolete",
            "hostFingerprint": "unused",
            "hostFingerprintHost": "unused.test",
            "hostFingerprintPort": 22,
            "allowedMethods": ["GET"],
            "allowedPathPrefixes": ["/old"],
            "cli": {"executablePath": "old-tool", "actions": []},
            "browser": {"origin": "https://old.test"},
            "credential": {"boundExecutable": "old-tool"}
        });
        fixture["connections"][0]
            .as_object_mut()
            .unwrap()
            .extend(obsolete.as_object().unwrap().clone());
        let old_bytes = serde_json::to_vec_pretty(&fixture).unwrap();
        fs::write(&path, &old_bytes).unwrap();
        drop(vault);

        let reopened = Vault::open(vault_dir.clone()).unwrap();
        let public = reopened.list_connections().unwrap();
        assert_eq!(public[0].capabilities, ["fill", "http"]);
        assert!(public[0].can_test);
        let backup = fs::read_dir(&vault_dir).unwrap().flatten().find(|entry| entry.file_name().to_string_lossy().starts_with("before-oci-modules-")).unwrap();
        assert_eq!(fs::read(backup.path().join("vault.json")).unwrap(), old_bytes);
        reopened.set_connection_enabled(saved.id, false).unwrap();
        let rewritten: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(rewritten["version"], VAULT_VERSION);
        for name in obsolete.as_object().unwrap().keys() {
            assert!(rewritten["connections"][0].get(name).is_none(), "{name}");
        }
        assert_eq!(rewritten["connections"][0]["encryptedSecrets"], encrypted);
        drop(reopened);
        let reopened = Vault::open(vault_dir).unwrap();
        assert_eq!(
            reopened
                .get_connection(saved.id)
                .unwrap()
                .secrets
                .get("apiCredential"),
            Some("existing-secret-marker")
        );
    }

    #[test]
    fn runtime_capabilities_and_saved_test_targets_are_distinct() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut input = ssh_input(None);
        input.modules.retain(|module| module.kind == "password");
        let saved = vault.save_connection(input).unwrap();
        assert_eq!(saved.capabilities, ["fill", "ssh", "http"]);
        assert!(!saved.can_test);

        let mut input = ssh_input(Some(saved.id));
        let saved = vault.save_connection(input.clone()).unwrap();
        assert!(saved.can_test);
        let stored = vault.get_connection(saved.id).unwrap();
        assert_eq!(stored.stored.test_target(&stored.secrets), Some("ssh"));

        input.modules.retain(|module| module.kind != "port");
        input.modules.push(ItemModule {
            id: uuid::Uuid::nil(),
            kind: "url".into(),
            value: "https://example.test".into(),
            ..Default::default()
        });
        let saved = vault.save_connection(input).unwrap();
        assert!(saved.can_test);
        let stored = vault.get_connection(saved.id).unwrap();
        assert_eq!(stored.stored.test_target(&stored.secrets), Some("http"));
    }

    #[test]
    fn reading_an_existing_browser_secret_does_not_rewrite_the_vault() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let secret = vault.browser_bridge_secret().unwrap();
        let path = vault.data_dir().join("vault.json");
        // A normal write prettifies the document; preserve compact bytes to
        // detect even a rewrite that leaves all logical values unchanged.
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let compact = serde_json::to_vec(&document).unwrap();
        fs::write(&path, &compact).unwrap();

        assert_eq!(vault.browser_bridge_secret().unwrap(), secret);
        assert_eq!(fs::read(path).unwrap(), compact);
    }

    #[test]
    fn editor_draft_encrypts_the_entire_form_and_can_be_reopened() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        let mut input = ssh_input(None);
        input.name = "unfinished-draft-name-7129".to_owned();
        input.description = "unfinished-draft-note-7129".to_owned();
        input.modules = vec![ItemModule {
            alternate_values: Vec::new(),
            id: uuid::Uuid::nil(),
            kind: "password".to_owned(),
            name: String::new(),
            value: String::new(),
            agent_visible: None,
        }];
        input.secrets.password = Some("unfinished-draft-secret-7129".to_owned());

        let saved = vault.save_editor_draft(None, input).unwrap();
        let contents = fs::read_to_string(vault_dir.join("vault.json")).unwrap();
        assert!(!contents.contains("unfinished-draft-name-7129"));
        assert!(!contents.contains("unfinished-draft-note-7129"));
        assert!(!contents.contains("unfinished-draft-secret-7129"));

        let drafts = vault.list_editor_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].id, saved.id);
        assert_eq!(drafts[0].input.name, "unfinished-draft-name-7129");
        assert_eq!(
            drafts[0].input.secrets.password.as_deref(),
            Some("unfinished-draft-secret-7129")
        );

        let existing_item_id = Uuid::new_v4();
        let mut replacement = ssh_input(Some(existing_item_id));
        replacement.name = "replacement-draft".to_owned();
        let replacement = vault.save_editor_draft(None, replacement).unwrap();
        let drafts = vault.list_editor_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].id, replacement.id);
        assert_eq!(drafts[0].input.id, Some(existing_item_id));
        assert_ne!(replacement.id, saved.id);

        vault.delete_editor_draft(replacement.id).unwrap();
        let drafts = vault.list_editor_drafts().unwrap();
        assert!(drafts.is_empty());
    }

    #[test]
    fn automatic_api_requires_only_a_secret_and_normalizes_optional_url() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut input = api_input(None, "Addressless API", "opaque-api-secret");
        input.http_auth_type = "auto".to_owned();
        input.modules.retain(|module| module.kind != "url");
        let addressless = vault.save_connection(input).unwrap();
        assert_eq!(addressless.name, "Addressless API");
        assert!(addressless.base_url.is_empty());
        assert_eq!(addressless.http_auth_type, "bearer");

        let mut input = api_input(None, "Example", "another-api-secret");
        input.http_auth_type = "auto".to_owned();
        input
            .modules
            .iter_mut()
            .find(|module| module.kind == "url")
            .unwrap()
            .value = "api.example.test/v1".to_owned();
        let normalized = vault.save_connection(input).unwrap();
        assert_eq!(normalized.base_url, "https://api.example.test/v1");

        let mut input = api_input(None, "Query API", "query-api-secret");
        input
            .modules
            .iter_mut()
            .find(|module| module.kind == "url")
            .unwrap()
            .value = "http://api.example.test/v1?preview=true".to_owned();
        let query_url = vault.save_connection(input).unwrap();
        assert_eq!(
            query_url.base_url,
            "http://api.example.test/v1?preview=true"
        );
    }

    #[test]
    fn url_username_and_password_automatically_enable_basic_http() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut input = ssh_input(None);
        input.name = "Basic HTTP".to_owned();
        input.http_auth_type = "auto".to_owned();
        input
            .modules
            .retain(|module| !matches!(module.kind.as_str(), "host" | "port"));
        input.modules.push(ItemModule {
            id: uuid::Uuid::nil(),
            kind: "url".into(),
            value: "api.example.test/v1".into(),
            ..Default::default()
        });

        let saved = vault.save_connection(input).unwrap();
        assert_eq!(saved.capabilities, vec!["fill", "ssh", "http"]);
        assert_eq!(saved.http_auth_type, "basic");
        assert_eq!(saved.base_url, "https://api.example.test/v1");
    }

    #[test]
    fn explicit_api_transport_settings_survive_editor_saves() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut input = api_input(None, "Configured API", "configured-api-secret");
        input.auth_prefix = "Token".to_owned();
        let saved = vault.save_connection(input).unwrap();

        assert_eq!(saved.http_auth_type, "bearer");
        assert_eq!(saved.auth_header, "X-API-Key");
        assert_eq!(saved.auth_prefix, "Token");
        assert_eq!(saved.test_path, "/health");
    }

    #[test]
    fn operational_api_values_are_preserved_instead_of_silently_truncated() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let long_url = format!("https://example.test/{}", "route/".repeat(120));
        let long_test_path = format!("/{}", "health/".repeat(80));
        let long_auth_prefix = "CustomAuthorizationPrefix".repeat(4);
        let mut input = api_input(None, "Long API configuration", "long-api-secret");
        input
            .modules
            .iter_mut()
            .find(|module| module.kind == "url")
            .unwrap()
            .value = long_url.clone();
        input.test_path = long_test_path.clone();
        input.auth_prefix = long_auth_prefix.clone();

        let saved = vault.save_connection(input).unwrap();
        assert_eq!(saved.base_url, long_url);
        assert_eq!(saved.test_path, long_test_path);
        assert_eq!(saved.auth_prefix, long_auth_prefix);
    }

    #[test]
    fn modules_derive_mixed_actions_and_keep_incomplete_items_as_drafts() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut mixed = ssh_input(None);
        mixed.http_auth_type = "auto".to_owned();
        mixed.modules = vec![
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "host".into(),
                name: String::new(),
                value: "127.0.0.1".into(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "port".into(),
                name: String::new(),
                value: "22".into(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "username".into(),
                name: String::new(),
                value: String::new(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "password".into(),
                name: String::new(),
                value: String::new(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "apiCredential".into(),
                name: String::new(),
                value: String::new(),
                agent_visible: None,
            },
        ];
        mixed
            .secrets
            .named_secrets
            .insert("apiCredential".into(), "mixed-api-marker".into());
        let saved = vault.save_connection(mixed).unwrap();
        assert_eq!(saved.capabilities, vec!["fill", "ssh", "http"]);
        assert!(
            saved
                .modules
                .iter()
                .all(|module| module.value != "mixed-api-marker")
        );

        let mut draft = ssh_input(None);
        draft.name = "Draft host".to_owned();
        draft.secrets = SecretBundle::default();
        draft.modules = vec![
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "host".into(),
                name: String::new(),
                value: "draft.example".into(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "port".into(),
                name: String::new(),
                value: "22".into(),
                agent_visible: None,
            },
            ItemModule {
                alternate_values: Vec::new(),
                id: uuid::Uuid::nil(),
                kind: "username".into(),
                name: String::new(),
                value: String::new(),
                agent_visible: None,
            },
        ];
        let saved_draft = vault.save_connection(draft).unwrap();
        assert!(saved_draft.capabilities.is_empty());
        assert_eq!(saved_draft.name, "Draft host");
    }

    #[test]
    fn new_items_require_a_name_and_at_least_one_module() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();

        let mut missing_name = ssh_input(None);
        missing_name.name.clear();
        missing_name.modules = vec![ItemModule {
            alternate_values: Vec::new(),
            id: uuid::Uuid::nil(),
            kind: "password".into(),
            name: String::new(),
            value: String::new(),
            agent_visible: None,
        }];
        assert!(
            vault
                .save_connection(missing_name)
                .unwrap_err()
                .to_string()
                .contains("name")
        );

        let mut missing_modules = ssh_input(None);
        missing_modules.modules.clear();
        assert!(
            vault
                .save_connection(missing_modules)
                .unwrap_err()
                .to_string()
                .contains("module")
        );
    }

    #[test]
    fn item_names_are_unique_ignoring_case_and_outer_whitespace() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let saved = vault
            .save_connection(api_input(None, "Production API", "first-token"))
            .unwrap();

        let duplicate = vault
            .save_connection(api_input(None, "  production api  ", "second-token"))
            .unwrap_err();
        assert!(duplicate.to_string().contains("name already exists"));

        let renamed = vault
            .save_connection(api_input(Some(saved.id), "PRODUCTION API", "first-token"))
            .unwrap();
        assert_eq!(renamed.name, "PRODUCTION API");
    }

    #[test]
    fn unsupported_vault_versions_are_rejected() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        let mut document = vault.read_document().unwrap();
        document.version = 6;
        vault.write_document_unlocked(&document).unwrap();
        drop(vault);

        let error = Vault::open(vault_dir).err().expect("version 6 must fail");
        assert!(format!("{error:#}").contains("supports vault versions 7 through 18"));
    }

    #[test]
    fn removing_a_secret_module_deletes_its_encrypted_value() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let saved = vault.save_connection(ssh_input(None)).unwrap();
        let existing = vault.get_connection(saved.id).unwrap();
        let mut update = ssh_input(Some(saved.id));
        update.secrets = SecretBundle::default();
        update.modules = existing
            .stored
            .modules
            .into_iter()
            .filter(|module| module.kind != "password")
            .collect();
        vault.save_connection(update).unwrap();
        let after = vault.get_connection(saved.id).unwrap();
        assert!(after.secrets.get("password").is_none());
        assert!(!after.stored.has_capability(&after.secrets, "ssh"));
    }

    #[test]
    fn owner_pin_is_hashed_and_owner_view_returns_only_on_explicit_request() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        vault.set_owner_pin("123456").unwrap();
        let saved = vault
            .save_connection(api_input(None, "owner-view", "owner-token-marker-7721"))
            .unwrap();

        assert!(vault.verify_owner_pin("123456").unwrap());
        assert!(!vault.verify_owner_pin("654321").unwrap());
        let raw = fs::read_to_string(vault_dir.join("vault.json")).unwrap();
        assert!(!raw.contains("123456"));
        assert!(!raw.contains("owner-token-marker-7721"));
        let public = serde_json::to_string(&vault.list_connections().unwrap()).unwrap();
        assert!(!public.contains("owner-token-marker-7721"));
        let owner = vault.owner_secret_view(saved.id).unwrap();
        assert_eq!(
            owner
                .fields
                .iter()
                .find(|field| field.name == "apiCredential")
                .map(|field| field.value.as_str()),
            Some("owner-token-marker-7721")
        );

        vault.disable_owner_pin().unwrap();
        assert!(!vault.owner_pin_configured().unwrap());
    }

    #[test]
    fn settings_patches_do_not_overwrite_unrelated_concurrent_changes() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().to_path_buf()).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));

        let language_vault = vault.clone();
        let language_barrier = barrier.clone();
        let language = std::thread::spawn(move || {
            language_barrier.wait();
            language_vault
                .update_settings_patch(SettingsPatch {
                    language: Some("en".to_owned()),
                    ..SettingsPatch::default()
                })
                .unwrap();
        });

        let browser_vault = vault.clone();
        let browser_barrier = barrier.clone();
        let browser = std::thread::spawn(move || {
            browser_barrier.wait();
            browser_vault
                .update_settings_patch(SettingsPatch {
                    browser_enabled: Some(true),
                    ..SettingsPatch::default()
                })
                .unwrap();
        });

        barrier.wait();
        language.join().unwrap();
        browser.join().unwrap();

        let settings = vault.settings().unwrap();
        assert_eq!(settings.language, "en");
        assert!(settings.browser_enabled);
    }

    #[test]
    fn concurrent_writes_keep_every_connection() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let handles = (0..4)
            .map(|index| {
                let vault = vault.clone();
                std::thread::spawn(move || {
                    vault
                        .save_connection(api_input(
                            None,
                            &format!("connection-{index}"),
                            &format!("secret-token-{index}"),
                        ))
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(vault.list_connections().unwrap().len(), 4);
    }

    #[test]
    fn concurrent_first_open_uses_one_initialized_vault() {
        let directory = tempdir().unwrap();
        let vault_dir = Arc::new(directory.path().join("vault"));
        let barrier = Arc::new(std::sync::Barrier::new(6));
        let handles = (0..6)
            .map(|index| {
                let vault_dir = vault_dir.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let vault = Vault::open((*vault_dir).clone()).unwrap();
                    vault
                        .save_connection(api_input(
                            None,
                            &format!("first-open-{index}"),
                            &format!("first-open-secret-{index}"),
                        ))
                        .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let reopened = Vault::open((*vault_dir).clone()).unwrap();
        let connections = reopened.list_decrypted_connections().unwrap();
        assert_eq!(connections.len(), 6);
        for index in 0..6 {
            assert!(connections.iter().any(|connection| {
                connection.stored.name == format!("first-open-{index}")
                    && connection.secrets.get("apiCredential")
                        == Some(format!("first-open-secret-{index}").as_str())
            }));
        }
    }

    #[test]
    fn imported_private_key_survives_source_file_removal() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let key_path = directory.path().join("id_test");
        let key_marker =
            "-----BEGIN PRIVATE KEY-----\nprivate-key-marker-4382\n-----END PRIVATE KEY-----";
        fs::write(&key_path, key_marker).unwrap();
        let vault = Vault::open(vault_dir.clone()).unwrap();
        let saved = vault
            .save_connection(ConnectionInput {
                certificate_authority: None,
                aws_bindings: None,
                oci_bindings: None,
                ssh_jump_item_id: None,
                ssh_jump_unresolved: false,
                ssh_auth_type: String::new(),
                login_uris: Vec::new(),
                item_type: String::new(),
                folder: String::new(),
                favorite: false,
                id: None,
                modules: vec![
                    ItemModule {
                        id: uuid::Uuid::nil(),
                        kind: "host".into(),
                        value: "127.0.0.1".into(),
                        ..Default::default()
                    },
                    ItemModule {
                        id: uuid::Uuid::nil(),
                        kind: "port".into(),
                        value: "22".into(),
                        ..Default::default()
                    },
                    ItemModule {
                        id: uuid::Uuid::nil(),
                        kind: "username".into(),
                        ..Default::default()
                    },
                    ItemModule {
                        id: uuid::Uuid::nil(),
                        kind: "privateKey".into(),
                        ..Default::default()
                    },
                ],
                name: "key-test".into(),
                enabled: true,
                description: String::new(),
                http_auth_type: String::new(),
                private_key_import_path: key_path.to_string_lossy().into_owned(),
                auth_header: String::new(),
                auth_location: String::new(),
                auth_prefix: String::new(),
                api_auth_headers: vec![],
                test_path: String::new(),
                remove_secret_names: vec![],
                secrets: {
                    let mut secrets = SecretBundle::default();
                    secrets
                        .named_secrets
                        .insert("username".into(), "root".into());
                    secrets
                },
            })
            .unwrap();
        fs::remove_file(&key_path).unwrap();

        let decrypted = vault.get_connection(saved.id).unwrap();
        assert_eq!(decrypted.secrets.private_key.as_deref(), Some(key_marker));
        assert_eq!(decrypted.stored.private_key_name, "id_test");
        let raw = fs::read_to_string(vault_dir.join("vault.json")).unwrap();
        assert!(!raw.contains("private-key-marker-4382"));
    }

    #[test]
    fn named_secrets_are_encrypted_preserved_on_empty_and_explicitly_deleted() {
        let directory = tempdir().unwrap();
        let vault_dir = directory.path().join("vault");
        let vault = Vault::open(vault_dir.clone()).unwrap();
        let mut secrets = SecretBundle::default();
        secrets
            .named_secrets
            .insert("password".into(), "browser-secret-marker-7482".into());
        secrets
            .named_secrets
            .insert("username".into(), "developer@example.test".into());
        let input = |id, secrets, remove_secret_names| ConnectionInput {
            certificate_authority: None,
            aws_bindings: None,
            oci_bindings: None,
            ssh_jump_item_id: None,
            ssh_jump_unresolved: false,
            ssh_auth_type: String::new(),
            login_uris: Vec::new(),
            item_type: String::new(),
            folder: String::new(),
            favorite: false,
            id,
            modules: vec![
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "username".into(),
                    ..Default::default()
                },
                ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    ..Default::default()
                },
            ],
            name: "generic-login".into(),
            enabled: true,
            description: String::new(),
            http_auth_type: String::new(),
            private_key_import_path: String::new(),
            auth_header: String::new(),
            auth_location: String::new(),
            auth_prefix: String::new(),
            api_auth_headers: vec![],
            test_path: String::new(),
            remove_secret_names,
            secrets,
        };
        let saved = vault.save_connection(input(None, secrets, vec![])).unwrap();
        assert!(
            !fs::read_to_string(vault_dir.join("vault.json"))
                .unwrap()
                .contains("browser-secret-marker-7482")
        );

        vault
            .save_connection(input(Some(saved.id), SecretBundle::default(), vec![]))
            .unwrap();
        assert_eq!(
            vault
                .get_connection(saved.id)
                .unwrap()
                .secrets
                .named_secrets
                .get("password")
                .map(String::as_str),
            Some("browser-secret-marker-7482")
        );

        vault
            .save_connection(input(
                Some(saved.id),
                SecretBundle::default(),
                vec!["password".into()],
            ))
            .unwrap();
        assert!(
            vault
                .get_connection(saved.id)
                .unwrap()
                .secrets
                .named_secrets
                .get("password")
                .is_none()
        );
    }

    #[test]
    fn secret_lookup_never_aliases_a_longer_module_name() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let module_name = format!("a{}", "x".repeat(79));
        let mut input = api_input(None, "Exact module lookup", "api-secret");
        input.modules.push(ItemModule {
            id: uuid::Uuid::nil(),
            kind: "customSecret".into(),
            name: module_name.clone(),
            agent_visible: Some(false),
            ..Default::default()
        });
        input
            .secrets
            .named_secrets
            .insert(module_name.clone(), "exact-secret".into());
        let saved = vault.save_connection(input).unwrap();

        assert_eq!(
            vault.get_secret_value(saved.id, &module_name).unwrap().2,
            "exact-secret"
        );
        assert!(
            vault
                .get_secret_value(saved.id, &format!("{module_name}x"))
                .is_err()
        );
    }

    #[test]
    fn custom_secret_modules_accept_natural_names() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let module_name = "Backup verification code";
        let mut input = api_input(None, "Natural module name", "api-secret");
        input.modules.push(ItemModule {
            id: uuid::Uuid::nil(),
            kind: "customSecret".into(),
            name: module_name.into(),
            agent_visible: Some(false),
            ..Default::default()
        });
        input
            .secrets
            .named_secrets
            .insert(module_name.into(), "custom-value".into());
        let saved = vault.save_connection(input).unwrap();

        assert_eq!(
            vault.get_secret_value(saved.id, module_name).unwrap().2,
            "custom-value"
        );
    }
}

fn advance_revisions(before: &VaultDocument, after: &mut VaultDocument) {
    for item in &mut after.connections {
        if let Some(old) = before.connections.iter().chain(before.trash.iter().map(|entry| &entry.connection)).find(|old| old.id == item.id) {
            if serde_json::to_value(&*item).ok() != serde_json::to_value(old).ok() {
                item.revision = old.revision.saturating_add(1);
            }
        } else { item.revision = item.revision.max(1); }
    }
}

pub fn ssh_config_complete(item: &StoredConnection, secrets: &SecretBundle) -> bool {
    !item.host.trim().is_empty() && item.port > 0 && secrets.get("username").is_some()
        && match item.ssh_auth_type.as_str() {
            "privateKey" => secrets.get("privateKey").is_some(),
            "certificate" => secrets.get("privateKey").is_some() && item.modules.iter().filter(|module| module.kind == "sshCertificate" && !module.value.trim().is_empty()).count() == 1,
            "" | "password" => secrets.get("password").is_some(),
            _ => false,
        }
}

pub fn ssh_endpoint_host(host: &str) -> Result<String> {
    let host = host.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
    anyhow::ensure!(!host.is_empty() && !host.chars().any(|ch| ch.is_control() || ch.is_whitespace()), "Invalid SSH host");
    Ok(host)
}
