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
    /// Other host or service URL addresses. Omit to preserve; an empty list clears them.
    pub alternate_values: Option<Vec<String>>,
}
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LoginRule {
    pub uri: String,
    pub match_type: String,
}
#[derive(Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OciBindingsPatch {
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub region: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub service_url: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub tenancy_ocid: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub user_ocid: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub fingerprint: Option<Option<String>>,
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub private_key: Option<Option<String>>,
}
impl OciBindingsPatch {
    fn apply(self, bindings: &mut crate::model::OciModuleBindings) -> Result<()> {
        if let Some(value) = self.region {
            bindings.region = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        if let Some(value) = self.service_url {
            bindings.service_url = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        if let Some(value) = self.tenancy_ocid {
            bindings.tenancy_ocid = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        if let Some(value) = self.user_ocid {
            bindings.user_ocid = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        if let Some(value) = self.fingerprint {
            bindings.fingerprint = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        if let Some(value) = self.private_key {
            bindings.private_key = value
                .map(|id| {
                    Uuid::parse_str(&id)
                        .map_err(|_| anyhow::anyhow!("Invalid OCI module binding ID"))
                })
                .transpose()?;
        }
        Ok(())
    }
}
fn nullable_oci_bindings<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Option<OciBindingsPatch>>, D::Error> {
    Option::<OciBindingsPatch>::deserialize(deserializer).map(Some)
}
#[derive(Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateInput {
    pub oci_bindings: Option<crate::model::OciModuleBindings>,
    /// Existing enabled direct SSH project ID. Omit for direct connection.
    pub ssh_jump_item_id: Option<String>,
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
    #[serde(default, deserialize_with = "nullable_oci_bindings")]
    pub oci_bindings: Option<Option<OciBindingsPatch>>,
    /// Omit to preserve the route, null to clear it, or an existing direct SSH project ID.
    #[serde(default, deserialize_with = "nullable_jump_id")]
    pub ssh_jump_item_id: Option<Option<String>>,
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
fn nullable_jump_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> std::result::Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(deserializer).map(Some)
}

fn jump_id(value: Option<String>) -> Result<Option<Uuid>> {
    value.map(|id| Uuid::parse_str(&id).map_err(|_| anyhow::anyhow!("Invalid SSH jump project ID"))).transpose()
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
        settings.mcp_writes_enabled()
            && !folder.is_empty()
            && settings.agent_write_folders.iter().any(|f| f == folder),
        "MCP write denied: enable writing and authorize the folder in desktop settings or permissions CLI"
    );
    Ok(())
}
impl Vault {
    pub fn write_capabilities(&self) -> Result<WriteCapabilities> {
        let settings = self.settings()?;
        Ok(WriteCapabilities {
            enabled:settings.mcp_writes_enabled(), folders:settings.agent_write_folders,
            operations:vec!["create".into(),"update".into(),"move via update".into()],
            item_types:vec!["ssh".into(),"api".into(),"login".into(),"custom".into()],
            modules:["host","port","username","password","privateKey","passphrase","url","apiCredential","totp","customSecret"].map(str::to_owned).to_vec(),
            ssh_auth_types:vec!["password".into(),"privateKey".into()], http_auth_types:vec!["bearer".into(),"apiKey".into(),"basic".into(),"oci-signature".into()],
            rules:vec![
                "sshJumpItemId selects an existing enabled direct SSH project. Update omits it to preserve or uses null to clear. Single-hop only; host trust is device-local and cannot be modified through MCP.".into(),
                "Create requires name, itemType, folder and modules; update requires item ID and expectedRevision. Omitted fields are preserved.".into(),
                "Host and url modules use value as the default address and alternateValues as other addresses. Omitted alternateValues is preserved; [] clears it. To switch defaults, submit the new value and remaining addresses together. No automatic failover. SSH addresses share the project port and credentials.".into(),
                "New modules require kind and value; updates require module ID. port is an integer 1..65535; other values are strings. customSecret also requires a unique name.".into(),
                "SSH requires username plus password/privateKey and explicit sshAuthType. API requires url plus explicit httpAuthType: bearer/apiKey use apiCredential; basic uses username/password; apiKey also requires authHeader.".into(),
                "New secrets are hidden. Cannot change visibility, delete items/modules, or configure write permissions. Never repeat submitted secrets.".into(),
                "Moves require both current and destination folders. Revision conflicts require a fresh query; never automatically replay stale writes.".into(),
                "Encrypted private keys require their passphrase in the same request. Unencrypted replacements clear the old passphrase.".into(),
                "OCI uses httpAuthType oci-signature and ociBindings, a map from authentication fields to stable module IDs. Create the modules first, then update ociBindings using returned module IDs and the latest revision. Omitted bindings preserve values; null clears a binding. OCI modules can be edited and signing keys replaced under the same folder and revision rules; visibility cannot change. Missing bindings may be saved, but signed execution requires every bound field.".into(),
            ],
        })
    }
    pub fn agent_create(&self, request: CreateInput) -> Result<WriteOutput> {
        self.update_document_with_result(|document| {
            authorize(&document.settings, request.folder.trim())?;
            let mut input: ConnectionInput=serde_json::from_value(serde_json::json!({"name":request.name,"itemType":request.item_type,"folder":request.folder.trim(),"description":request.description.unwrap_or_default(),"enabled":request.enabled.unwrap_or(true)}))?;
            input.oci_bindings = request.oci_bindings;
            input.ssh_jump_item_id = jump_id(request.ssh_jump_item_id)?;
            set_auth(&mut input,request.ssh_auth_type,request.http_auth_type,request.auth_header)?;
            if let Some(rules)=request.login_uris { input.login_uris=login_rules(rules)?; }
            apply_modules(&mut input, &request.modules)?;
            validate_credentials(&mut input, None, &request.modules)?;
            validate_item(&input, input.http_auth_type == crate::oci::AUTH_MODE)?;
            let key=input.secrets.private_key.clone().map(zeroize::Zeroizing::new);
            let saved=self.save_connection_in_document(document,input,None,key.as_ref().map(|k|(k.as_str(),"SSH KEY"))).map_err(|_|anyhow::anyhow!("Invalid item or module configuration; nothing was saved"))?;
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
            let secrets = self.decrypt_stored(&existing)?;
            let mut input: ConnectionInput =
                serde_json::from_value(serde_json::to_value(existing.portable())?)?;
            input.secrets = secrets.clone();
            if let Some(patch) = request.oci_bindings {
                match patch {
                    None => input.oci_bindings = None,
                    Some(patch) => { patch.apply(input.oci_bindings.get_or_insert_with(Default::default))?; }
                }
            }
            if let Some(value) = request.ssh_jump_item_id {
                input.ssh_jump_item_id = jump_id(value)?;
                input.ssh_jump_unresolved = false;
            }
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
            apply_modules(&mut input, &request.modules)?;
            validate_credentials(&mut input, Some(&existing), &request.modules)?;
            validate_item(&input, input.http_auth_type == crate::oci::AUTH_MODE)?;
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
            matches!(mode.as_str(), "bearer" | "apiKey" | "basic" | "oci-signature"),
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
fn apply_modules(input: &mut ConnectionInput, patches: &[ModulePatch]) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let original_secrets = input.secrets.clone();
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
            ensure!(
                kind != "customSecret"
                    || patch
                        .name
                        .as_ref()
                        .is_some_and(|name| !name.trim().is_empty()),
                "Custom field requires a name"
            );
            input.modules.push(ItemModule {
                alternate_values: Vec::new(),
                id: Uuid::new_v4(),
                kind: kind.into(),
                name: patch
                    .name
                    .as_ref()
                    .map(|name| name.trim().to_owned())
                    .unwrap_or_else(|| kind.into()),
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
        if let Some(addresses) = &patch.alternate_values {
            ensure!(matches!(module.kind.as_str(), "host" | "url"), "Only host and url modules support other addresses");
            module.alternate_values = addresses.clone();
        }
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
            let name = patch.name.as_ref().unwrap().trim().to_owned();
            ensure!(!name.trim().is_empty(), "Custom field requires a name");
            if let Some(value) = original_secrets.get(&old).map(str::to_owned) {
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
    // Renames are simultaneous: an old name reused by another final module is not removed.
    let retained: Vec<String> = input
        .modules
        .iter()
        .filter_map(|m| m.secret_name().map(str::to_owned))
        .collect();
    input
        .remove_secret_names
        .retain(|name| !retained.contains(name));
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
    if input.http_auth_type == crate::oci::AUTH_MODE {
        crate::oci::validate_modules(&input.modules, &input.secrets, input.oci_bindings.as_ref())?;
        return Ok(());
    }
    let key_changed = changed("privateKey");
    let pass_changed = changed("passphrase");
    if key_changed || pass_changed {
        let key =
            input.secrets.private_key.as_deref().ok_or_else(|| {
                anyhow::anyhow!("Private key is required for passphrase validation")
            })?;
        if key_changed && (crate::ssh_keys::decode_private_key(key, None).is_ok() || crate::oci::validate_pem(key).is_ok()) {
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
    ensure!(matches!(input.item_type.as_str(), "ssh" | "api" | "login" | "custom"), "Unsupported project type for MCP writing");
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
