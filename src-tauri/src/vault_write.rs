//! MCP-only write adapter. Desktop saves retain their existing authorization boundary.
use crate::{
    model::{ConnectionInput, ItemModule, PublicConnection, Settings, StoredConnection},
    vault::Vault,
};
use anyhow::{Result, bail, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModulePatch {
    /// Existing module ID for updates; omit to add a module.
    pub id: Option<String>,
    /// Required for new modules. Existing module kinds cannot be changed in place.
    pub kind: Option<String>,
    pub name: Option<String>,
    /// Text value, or an integer for port. Hidden values must not be repeated in chat.
    pub value: Option<serde_json::Value>,
}
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRule {
    pub uri: String,
    pub match_type: String,
}
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInput {
    pub name: String,
    pub item_type: String,
    pub folder: String,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub ssh_auth_type: Option<String>,
    pub http_auth_type: Option<String>,
    pub auth_header: Option<String>,
    pub login_uris: Option<Vec<LoginRule>>,
    pub modules: Vec<ModulePatch>,
}
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UpdateInput {
    pub id: String,
    pub expected_revision: u64,
    pub name: Option<String>,
    pub item_type: Option<String>,
    pub folder: Option<String>,
    pub description: Option<String>,
    pub enabled: Option<bool>,
    pub ssh_auth_type: Option<String>,
    pub http_auth_type: Option<String>,
    pub auth_header: Option<String>,
    pub login_uris: Option<Vec<LoginRule>>,
    #[serde(default)]
    pub modules: Vec<ModulePatch>,
}
#[derive(Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteOutput {
    pub id: String,
    pub revision: u64,
    pub module_ids: Vec<String>,
    pub status: String,
}
impl From<PublicConnection> for WriteOutput {
    fn from(item: PublicConnection) -> Self {
        Self {
            id: item.id.to_string(),
            revision: item.revision,
            module_ids: item.modules.iter().map(|m| m.id.to_string()).collect(),
            status: "success".into(),
        }
    }
}
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteCapabilities {
    pub enabled: bool,
    pub folders: Vec<String>,
    pub operations: Vec<String>,
    pub item_types: Vec<String>,
    pub modules: Vec<String>,
    pub ssh_auth_types: Vec<String>,
    pub http_auth_types: Vec<String>,
    pub rules: Vec<String>,
}
pub(crate) fn normalize_folders(folders: Vec<String>) -> Result<Vec<String>> {
    let mut output = Vec::new();
    for folder in folders {
        let folder = folder.trim().to_owned();
        ensure!(
            !folder.is_empty()
                && folder.chars().count() <= 120
                && !folder.chars().any(char::is_control),
            "Invalid writable folder"
        );
        ensure!(!output.contains(&folder), "Duplicate writable folder");
        output.push(folder);
    }
    Ok(output)
}
fn authorize(settings: &Settings, folder: &str) -> Result<()> {
    ensure!(
        settings.agent_write_enabled
            && !folder.is_empty()
            && settings.agent_write_folders.iter().any(|f| f == folder),
        "MCP write denied: enable writing and authorize the folder in desktop settings"
    );
    Ok(())
}
impl Vault {
    pub fn write_capabilities(&self) -> Result<WriteCapabilities> {
        let settings = self.settings()?;
        Ok(WriteCapabilities {
            enabled:settings.agent_write_enabled && !settings.agent_write_folders.is_empty(), folders:settings.agent_write_folders,
            operations:vec!["create".into(),"update".into(),"move via update".into()],
            item_types:vec!["ssh".into(),"api".into(),"login".into(),"custom".into()],
            modules:["host","port","username","password","privateKey","passphrase","url","apiCredential","totp","customSecret"].map(str::to_owned).to_vec(),
            ssh_auth_types:vec!["password".into(),"privateKey".into()], http_auth_types:vec!["bearer".into(),"apiKey".into(),"basic".into()],
            rules:vec![
                "Create requires name, itemType, folder and modules; update requires item ID and expectedRevision. Omitted fields are preserved.".into(),
                "New modules require kind and value; updates require module ID. port is an integer 1..65535; other values are strings. customSecret also requires a unique name.".into(),
                "SSH requires username plus password/privateKey and explicit sshAuthType. API requires url plus explicit httpAuthType: bearer/apiKey use apiCredential; basic uses username/password; apiKey also requires authHeader.".into(),
                "New secrets are hidden. Cannot change visibility, delete items/modules, or configure write permissions. Never repeat submitted secrets.".into(),
                "Moves require both current and destination folders. Revision conflicts require a fresh query; never automatically replay stale writes.".into(),
                "Encrypted private keys require their passphrase in the same request. Unencrypted replacements clear the old passphrase.".into(),
                "OCI: only existing item metadata and independent ordinary modules can be patched. Signing configuration, derived URL, item type and auth mode cannot change; no OCI creation/conversion/signing-key replacement.".into(),
            ],
        })
    }
    pub fn agent_create(&self, request: CreateInput) -> Result<WriteOutput> {
        self.update_document_with_result(|document| {
            authorize(&document.settings, request.folder.trim())?;
            let mut input: ConnectionInput=serde_json::from_value(serde_json::json!({"name":request.name,"itemType":request.item_type,"folder":request.folder.trim(),"description":request.description.unwrap_or_default(),"enabled":request.enabled.unwrap_or(true)}))?;
            set_auth(&mut input,request.ssh_auth_type,request.http_auth_type,request.auth_header)?;
            if let Some(rules)=request.login_uris { input.login_uris=login_rules(rules)?; }
            apply_modules(&mut input, &request.modules, false)?;
            validate_credentials(&mut input, None, &request.modules)?;
            validate_item(&input, false)?;
            let key=input.secrets.private_key.clone().map(zeroize::Zeroizing::new);
            let saved=self.save_connection_in_document(document,input,None,key.as_ref().map(|k|(k.as_str(),"SSH KEY")),None,false).map_err(|_|anyhow::anyhow!("Invalid item or module configuration; nothing was saved"))?;
            Ok(saved.into())
        })
    }
    pub fn agent_update(&self, request: UpdateInput) -> Result<WriteOutput> {
        self.update_document_with_result(|document| {
            let id =
                Uuid::parse_str(&request.id).map_err(|_| anyhow::anyhow!("Invalid project ID"))?;
            let existing = document
                .connections
                .iter()
                .find(|item| item.id == id)
                .ok_or_else(|| anyhow::anyhow!("Project unavailable"))?
                .clone();
            authorize(&document.settings, &existing.folder)?;
            if let Some(folder) = &request.folder {
                authorize(&document.settings, folder.trim())?;
            }
            ensure!(
                request.expected_revision == existing.revision,
                "Project changed; query again before updating"
            );
            let is_oci = existing.http_auth_type == crate::oci::AUTH_MODE;
            if is_oci {
                ensure!(
                    request.item_type.is_none()
                        && request.http_auth_type.is_none()
                        && request.auth_header.is_none()
                        && request.login_uris.is_none(),
                    "OCI signing fields require the desktop OCI editor"
                );
            }
            let secrets = self.decrypt_stored(&existing)?;
            let mut input: ConnectionInput =
                serde_json::from_value(serde_json::to_value(existing.portable())?)?;
            input.secrets = secrets.clone();
            if let Some(v) = request.name {
                input.name = v;
            }
            if let Some(v) = request.item_type {
                input.item_type = v;
            }
            if let Some(v) = request.folder {
                input.folder = v.trim().into();
            }
            if let Some(v) = request.description {
                input.description = v;
            }
            if let Some(v) = request.enabled {
                input.enabled = v;
            }
            set_auth(
                &mut input,
                request.ssh_auth_type,
                request.http_auth_type,
                request.auth_header,
            )?;
            if let Some(rules) = request.login_uris {
                input.login_uris = login_rules(rules)?;
            }
            apply_modules(&mut input, &request.modules, is_oci)?;
            validate_credentials(&mut input, Some(&existing), &request.modules)?;
            validate_item(&input, is_oci)?;
            let key = input
                .secrets
                .private_key
                .clone()
                .map(zeroize::Zeroizing::new);
            let key_name = if existing.private_key_name.is_empty() {
                "SSH KEY"
            } else {
                &existing.private_key_name
            };
            let saved = self
                .save_connection_in_document(
                    document,
                    input,
                    None,
                    key.as_ref().map(|k| (k.as_str(), key_name)),
                    None,
                    is_oci,
                )
                .map_err(|_| {
                    anyhow::anyhow!("Invalid item or module configuration; nothing was saved")
                })?;
            Ok(saved.into())
        })
    }
}
fn login_rules(rules: Vec<LoginRule>) -> Result<Vec<crate::login_uri::LoginUri>> {
    serde_json::from_value(serde_json::Value::Array(
        rules
            .into_iter()
            .map(|r| serde_json::json!({"uri":r.uri,"matchType":r.match_type}))
            .collect(),
    ))
    .map_err(|_| anyhow::anyhow!("Invalid website matching rules"))
}
fn set_auth(
    input: &mut ConnectionInput,
    ssh: Option<String>,
    http: Option<String>,
    header: Option<String>,
) -> Result<()> {
    if let Some(mode) = ssh {
        ensure!(
            matches!(mode.as_str(), "password" | "privateKey"),
            "Invalid SSH authentication mode"
        );
        input.ssh_auth_type = mode;
    }
    if let Some(mode) = http {
        ensure!(
            matches!(mode.as_str(), "bearer" | "apiKey" | "basic"),
            "Unsupported API authentication mode"
        );
        input.http_auth_type = mode;
        input.auth_location = "header".into();
        input.auth_prefix = String::new();
        input.api_auth_headers.clear();
    }
    if let Some(header) = header {
        ensure!(
            reqwest::header::HeaderName::from_bytes(header.as_bytes()).is_ok(),
            "Invalid API header name"
        );
        ensure!(
            !matches!(
                header.to_ascii_lowercase().as_str(),
                "host" | "content-length" | "transfer-encoding" | "connection"
            ),
            "Unsupported API credential header"
        );
        input.auth_header = header;
    }
    Ok(())
}
fn apply_modules(input: &mut ConnectionInput, patches: &[ModulePatch], oci: bool) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    for patch in patches {
        let index = if let Some(id) = &patch.id {
            let id = Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("Invalid module ID"))?;
            ensure!(seen.insert(id), "Duplicate module update");
            input
                .modules
                .iter()
                .position(|m| m.id == id)
                .ok_or_else(|| anyhow::anyhow!("Module does not belong to this project"))?
        } else {
            let kind = patch
                .kind
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("New module requires kind"))?;
            ensure!(
                matches!(
                    kind,
                    "host"
                        | "port"
                        | "username"
                        | "password"
                        | "privateKey"
                        | "passphrase"
                        | "url"
                        | "apiCredential"
                        | "totp"
                        | "customSecret"
                ),
                "Unsupported module kind"
            );
            ensure!(patch.value.is_some(), "New module requires value");
            input.modules.push(ItemModule {
                id: Uuid::new_v4(),
                kind: kind.into(),
                name: patch.name.clone().unwrap_or_else(|| kind.into()),
                value: String::new(),
                agent_visible: None,
            });
            input.modules.len() - 1
        };
        let module = &mut input.modules[index];
        ensure!(
            patch.kind.as_ref().is_none_or(|kind| kind == &module.kind),
            "Cannot change existing module kind"
        );
        ensure!(
            !oci || module.kind != "url",
            "OCI URL is derived from its region"
        );
        if let Some(name) = &patch.name {
            ensure!(
                name == &module.name || module.kind == "customSecret",
                "Preset module names are fixed"
            );
        }
        // Renaming a secret must not strand its old value or visibility.
        if module.kind == "customSecret"
            && patch.name.as_ref().is_some_and(|name| name != &module.name)
        {
            let old = module.name.clone();
            let name = patch.name.as_ref().unwrap();
            ensure!(!name.trim().is_empty(), "Custom field requires a name");
            if let Some(value) = input.secrets.get(&old).map(str::to_owned) {
                input.secrets.named_secrets.insert(name.clone(), value);
            }
            input.remove_secret_names.push(old);
            module.name = name.clone();
        }
        if let Some(value) = &patch.value {
            let value = if module.kind == "port" {
                value
                    .as_u64()
                    .filter(|n| *n > 0 && *n <= 65535)
                    .map(|n| n.to_string())
                    .ok_or_else(|| anyhow::anyhow!("Port must be an integer from 1 to 65535"))?
            } else {
                value
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("Module value must be text"))?
                    .to_owned()
            };
            ensure!(value.len() <= 1024 * 1024, "Module value too large");
            if let Some(name) = module.secret_name() {
                ensure!(!value.is_empty(), "Secret value cannot be empty");
                match name {
                    "password" => input.secrets.password = Some(value),
                    "privateKey" => input.secrets.private_key = Some(value),
                    "passphrase" => input.secrets.passphrase = Some(value),
                    _ => {
                        input.secrets.named_secrets.insert(name.into(), value);
                    }
                }
            } else {
                module.value = value;
            }
        }
    }
    Ok(())
}
fn validate_credentials(
    input: &mut ConnectionInput,
    _existing: Option<&StoredConnection>,
    patches: &[ModulePatch],
) -> Result<()> {
    let changed = |kind: &str| {
        patches.iter().any(|p| {
            p.value.is_some()
                && (p.kind.as_deref() == Some(kind)
                    || p.id.as_ref().is_some_and(|id| {
                        input
                            .modules
                            .iter()
                            .any(|m| m.id.to_string() == *id && m.kind == kind)
                    }))
        })
    };
    let key_changed = changed("privateKey");
    let pass_changed = changed("passphrase");
    if key_changed || pass_changed {
        let key =
            input.secrets.private_key.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Private key is required for passphrase validation")
            })?;
        if key_changed && crate::ssh_keys::decode_private_key(key, None).is_ok() {
            input.secrets.passphrase = None;
            input.secrets.named_secrets.remove("passphrase");
            input.remove_secret_names.push("passphrase".into());
        } else {
            ensure!(
                !key_changed || pass_changed,
                "Encrypted private key requires its passphrase in the same request"
            );
            crate::ssh_keys::decode_private_key(key, input.secrets.passphrase.as_deref())?;
        }
    }
    if changed_totp(input, patches) {
        crate::browser::current_totp(input.secrets.get("totp").unwrap_or_default())
            .map_err(|_| anyhow::anyhow!("Invalid TOTP configuration"))?;
    }
    Ok(())
}
fn changed_totp(input: &ConnectionInput, patches: &[ModulePatch]) -> bool {
    patches.iter().any(|p| {
        p.value.is_some()
            && (p.kind.as_deref() == Some("totp")
                || p.id.as_ref().is_some_and(|id| {
                    input
                        .modules
                        .iter()
                        .any(|m| m.id.to_string() == *id && m.kind == "totp")
                }))
    })
}
fn validate_item(input: &ConnectionInput, oci: bool) -> Result<()> {
    if !oci {
        ensure!(
            matches!(input.item_type.as_str(), "ssh" | "api" | "login" | "custom"),
            "Unsupported project type for MCP writing"
        );
    }
    let has = |name: &str| input.secrets.get(name).is_some_and(|s| !s.is_empty());
    if input.item_type == "ssh" {
        ensure!(
            matches!(input.ssh_auth_type.as_str(), "password" | "privateKey")
                && has("username")
                && has(&input.ssh_auth_type),
            "SSH requires an explicit authentication mode, username and its credential"
        );
        ensure!(
            input
                .modules
                .iter()
                .any(|m| m.kind == "host" && !m.value.trim().is_empty()),
            "SSH host is required"
        );
    }
    if input.item_type == "api" && !oci {
        ensure!(
            input
                .modules
                .iter()
                .any(|m| m.kind == "url" && !m.value.trim().is_empty()),
            "API URL is required"
        );
        match input.http_auth_type.as_str() {
            "bearer" => ensure!(has("apiCredential"), "API credential required"),
            "apiKey" => ensure!(
                has("apiCredential") && !input.auth_header.is_empty(),
                "API key and header name required"
            ),
            "basic" => ensure!(
                has("username") && has("password"),
                "Basic requires username and password"
            ),
            _ => bail!("Explicit API authentication mode required"),
        }
    }
    Ok(())
}
#[cfg(test)]
#[path = "vault_write_tests.rs"]
mod tests;
