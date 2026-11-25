use crate::{
    action_workflow::{
        ActionCapabilitiesInput, ActionResponse, HttpRemoteError, HttpResponseFormat,
        HttpPagination, HttpTarget, HttpToolResponse, MissingInput, NeedInputCode, NextCall, ProjectCandidate,
        SelectedProject, SelectionReason, TargetSource, UnavailableCode,
    },
    browser::{BrowserBridge, current_totp},
    desktop,
    executor::{
        ApiPreviewIssue, ApiRequestInput, ApiResponse, SshResponse, SshTransferResponse, describe_api_request,
        execute_api, execute_local, execute_ssh, preview_api_request, ssh_download, ssh_upload,
    },
    model::NewActivity,
    native_workflow::NativeWorkflow,
    terminal::{TerminalManager, TerminalOpenResult, TerminalReadResult},
    vault::{DecryptedConnection, Vault},
};
use anyhow::{Context, Result, bail};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    service::{MaybeSendFuture, NotificationContext, RoleServer},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct VaultMcp {
    vault: Vault,
    browser: BrowserBridge,
    terminal: TerminalManager,
    active_item: Arc<RwLock<Option<Uuid>>>,
    active_terminal: Arc<RwLock<Option<Uuid>>>,
    terminal_items: Arc<RwLock<HashMap<Uuid, Uuid>>>,
    client_name: Arc<RwLock<String>>,
    tool_router: ToolRouter<Self>,
}

enum ProjectMatch {
    Selected(DecryptedConnection, SelectionReason),
    Ambiguous(Vec<ProjectCandidate>),
    NotFound(Vec<ProjectCandidate>),
    NoCompatibleProject,
}

impl ProjectMatch {
    fn workflow_response(self) -> ActionResponse {
        match self {
            Self::Selected(connection, reason) => ActionResponse::Selected {
                project: SelectedProject::new(&connection.stored, reason),
            },
            Self::Ambiguous(candidates) => ActionResponse::ChooseProject {
                candidates,
                missing_inputs: vec![MissingInput { name: "item".to_owned(), source: "caller" }],
            },
            Self::NotFound(candidates) => ActionResponse::Unavailable {
                code: UnavailableCode::ProjectNotFound,
                project: None,
                candidates,
            },
            Self::NoCompatibleProject => ActionResponse::Unavailable {
                code: UnavailableCode::NoCompatibleProject,
                project: None,
                candidates: Vec::new(),
            },
        }
    }
}

impl std::fmt::Debug for VaultMcp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("VaultMcp").finish_non_exhaustive()
    }
}

impl VaultMcp {
    pub fn new(vault: Vault) -> Self {
        Self {
            browser: BrowserBridge::new(vault.clone()),
            terminal: TerminalManager::new(),
            active_item: Arc::new(RwLock::new(None)),
            active_terminal: Arc::new(RwLock::new(None)),
            terminal_items: Arc::new(RwLock::new(HashMap::new())),
            client_name: Arc::new(RwLock::new(String::new())),
            vault,
            tool_router: Self::tool_router(),
        }
    }

    fn activity_source(&self) -> String {
        let client_name = self
            .client_name
            .read()
            .map(|name| name.clone())
            .unwrap_or_default();
        if client_name.is_empty() {
            "MCP CLIENT".to_owned()
        } else {
            format!("MCP · {client_name}")
        }
    }

    fn bind_terminal_item(&self, session_id: Uuid, item_id: Uuid) {
        if let Ok(mut bindings) = self.terminal_items.write() {
            bindings.insert(session_id, item_id);
        }
    }

    fn bind_active_item(&self, item_id: Uuid) {
        if let Ok(mut active_item) = self.active_item.write() {
            *active_item = Some(item_id);
        }
    }

    fn clear_active_item(&self) {
        if let Ok(mut active_item) = self.active_item.write() {
            *active_item = None;
        }
    }

    fn bind_active_terminal(&self, session_id: Uuid) {
        if let Ok(mut active_terminal) = self.active_terminal.write() {
            *active_terminal = Some(session_id);
        }
    }

    fn clear_active_terminal(&self, session_id: Uuid) {
        if let Ok(mut active_terminal) = self.active_terminal.write()
            && *active_terminal == Some(session_id)
        {
            *active_terminal = None;
        }
    }

    fn resolve_terminal_session_id(
        &self,
        session_id: &str,
        require_running: bool,
    ) -> Result<Uuid, String> {
        let session_id = if session_id.trim().is_empty() {
            self.active_terminal
                .read()
                .ok()
                .and_then(|session_id| *session_id)
                .ok_or_else(|| {
                    "No current terminal session; call terminal_start first or provide sessionId"
                        .to_owned()
                })?
        } else {
            Uuid::parse_str(session_id).map_err(|_| "Invalid terminal session ID".to_owned())?
        };
        if !self
            .terminal
            .contains(session_id)
            .map_err(|error| error.to_string())?
        {
            self.clear_active_terminal(session_id);
            return Err("Terminal session not found; start a new terminal".to_owned());
        }
        if require_running
            && !self
                .terminal
                .is_running(session_id)
                .map_err(|error| error.to_string())?
        {
            self.clear_active_terminal(session_id);
            return Err("The current terminal session has ended; start a new terminal".to_owned());
        }
        self.bind_active_terminal(session_id);
        Ok(session_id)
    }

    fn active_connection_for_action(
        &self,
        capability: Option<&str>,
    ) -> Option<DecryptedConnection> {
        let item_id = self.active_item.read().ok().and_then(|item| *item)?;
        let connection = self.vault.agent_connection(item_id).ok()?;
        let supports_action = connection_supports_action(&connection, capability);
        (connection.stored.enabled && supports_action).then_some(connection)
    }

    fn unbind_terminal_item(&self, session_id: Uuid) {
        if let Ok(mut bindings) = self.terminal_items.write() {
            bindings.remove(&session_id);
        }
    }

    fn resolve_terminal_item(
        &self,
        session_id: Uuid,
        query: &str,
    ) -> Result<DecryptedConnection, String> {
        if !query.trim().is_empty() {
            return self.resolve_item_for_action(query, Some("fill"));
        }
        let bound = self
            .terminal_items
            .read()
            .ok()
            .and_then(|bindings| bindings.get(&session_id).copied());
        if let Some(item_id) = bound {
            let connection = self
                .vault
                .agent_connection(item_id)
                .map_err(|_| "The OpenKeylet project bound to this terminal is no longer available; specify item again".to_owned())?;
            if !connection.stored.enabled
                || !connection
                    .stored
                    .has_capability(&connection.secrets, "fill")
            {
                return Err("The OpenKeylet project bound to this terminal is no longer available; specify item again".to_owned());
            }
            return Ok(connection);
        }
        self.resolve_item_for_action("", Some("fill"))
    }

    async fn track<T, F>(&self, item_id: Uuid, action: String, future: F) -> Result<T, String>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        let started = Instant::now();
        let decrypted = self.vault.get_connection(item_id).ok();
        let item_name = decrypted
            .as_ref()
            .map(|connection| connection.stored.name.clone())
            .unwrap_or_else(|| "Unknown project".to_owned());
        let action = decrypted.as_ref().map_or(action.clone(), |connection| {
            crate::policy::redact(action, &connection.stored, &connection.secrets)
        });
        match future.await {
            Ok(value) => {
                let _ = self.vault.add_activity(NewActivity {
                    status: "success".to_owned(),
                    source: self.activity_source(),
                    connection_name: item_name,
                    action,
                    duration_ms: started.elapsed().as_millis() as u64,
                    error: String::new(),
                });
                Ok(value)
            }
            Err(error) => {
                let message = decrypted.as_ref().map_or_else(
                    || error.to_string(),
                    |connection| {
                        crate::policy::redact(
                            error.to_string(),
                            &connection.stored,
                            &connection.secrets,
                        )
                    },
                );
                let _ = self.vault.add_activity(NewActivity {
                    status: "error".to_owned(),
                    source: self.activity_source(),
                    connection_name: item_name,
                    action,
                    duration_ms: started.elapsed().as_millis() as u64,
                    error: message.clone(),
                });
                Err(message)
            }
        }
    }

    fn search_items(&self, input: ItemsSearchInput) -> Result<ItemsSearchOutput, String> {
        let query = input
            .query
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let mut connections = self
            .vault
            .list_decrypted_connections()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|decrypted| {
                decrypted.stored.enabled
                    && !decrypted.stored.capabilities(&decrypted.secrets).is_empty()
            })
            .collect::<Vec<_>>();

        if let Some(query) = query.as_deref() {
            connections = filter_connections_by_query(connections, query);
        }

        if connections.len() == 1 {
            self.bind_active_item(connections[0].stored.id);
        } else if query.is_some() {
            self.clear_active_item();
        }

        let items = connections
            .into_iter()
            .map(|decrypted| -> Result<ItemOutput, String> {
                let modules = decrypted
                    .stored
                    .modules
                    .iter()
                    .map(|module| -> Result<ItemModuleOutput, String> {
                        let agent_visible = module.agent_visible() && !(decrypted.stored.certificate_authority.is_some() && module.is_secret());
                        let configured = if let Some(name) = module.secret_name() {
                            decrypted.secrets.get(name).is_some()
                        } else {
                            !module.value.trim().is_empty()
                        };
                        let value = if agent_visible {
                            let mut value = if let Some(name) = module.secret_name() {
                                decrypted.secrets.get(name).unwrap_or_default().to_owned()
                            } else {
                                module.value.clone()
                            };
                            if module.kind == "totp" && !value.is_empty() {
                                value = current_totp(&value).map_err(|error| error.to_string())?;
                            }
                            Some(value)
                        } else {
                            None
                        };
                        Ok(ItemModuleOutput {
                            id: module.id.to_string(),
                            kind: module.kind.clone(),
                            name: module.name.clone(),
                            configured,
                            agent_visible,
                            alternate_values: if agent_visible && !module.is_secret() { Some(module.alternate_values.clone()) } else { None },
                            value,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let mut actions = Vec::new();
                if decrypted.stored.has_capability(&decrypted.secrets, "fill") {
                    actions.push("credential_fill".to_owned());
                }
                if decrypted.stored.has_capability(&decrypted.secrets, "ssh") {
                    actions.push("ssh_run".to_owned());
                    actions.push("ssh_upload".to_owned());
                    actions.push("ssh_download".to_owned());
                }
                if decrypted.stored.has_capability(&decrypted.secrets, "http") {
                    actions.push("http_send".to_owned());
                }
                if decrypted.stored.certificate_authority.is_some() {
                    actions.clear(); actions.push("certificate_issue".to_owned());
                }
                if decrypted.stored.modules.iter().any(|module| module.kind == "privateKey") { actions.push("certificate_check_key".to_owned()); }
                let oci_context = if decrypted.stored.certificate_authority.is_none() && decrypted.stored.http_auth_type == crate::oci::AUTH_MODE {
                    decrypted.stored.oci_bindings.as_ref().and_then(|bindings| {
                        let visible = |id| {
                            decrypted.stored.modules.iter()
                                .find(|module| Some(module.id) == id)
                                .filter(|module| module.agent_visible())
                                .and_then(|_| crate::oci::binding_value(&decrypted.stored.modules, &decrypted.secrets, id))
                                .map(str::to_owned)
                        };
                        let context = OciContext {
                            region: visible(bindings.region),
                            tenancy_ocid: visible(bindings.tenancy_ocid),
                        };
                        (context.region.is_some() || context.tenancy_ocid.is_some()).then_some(context)
                    })
                } else { None };
                Ok(ItemOutput {
                    certificate_authority: decrypted.stored.certificate_authority.clone(),
                    aws_bindings: decrypted.stored.aws_bindings.clone(),
                    api_service: if decrypted.stored.certificate_authority.is_none() { crate::api_provider::describe_item(&decrypted.stored, &decrypted.secrets).map_err(|error| error.to_string())? } else { None },
                    oci_bindings: decrypted.stored.oci_bindings.clone(),
                    ssh_jump_item_id: decrypted.stored.ssh_jump_item_id.map(|id|id.to_string()),
                    ssh_jump_unresolved: decrypted.stored.ssh_jump_unresolved,
                    id: decrypted.stored.id.to_string(),
                    revision: decrypted.stored.revision,
                    name: decrypted.stored.name,
                    modules,
                    actions,
                    oci_context,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ItemsSearchOutput { items })
    }

    fn resolve_item_for_action(
        &self,
        query: &str,
        capability: Option<&str>,
    ) -> Result<DecryptedConnection, String> {
        self.resolve_action_project(query, None, capability)
    }

    fn match_action_project(
        &self,
        query: &str,
        item_id: Option<Uuid>,
        capability: Option<&str>,
    ) -> Result<ProjectMatch, String> {
        self.match_action_project_where(query, item_id, capability, |_| true)
    }

    fn match_action_project_where(
        &self,
        query: &str,
        item_id: Option<Uuid>,
        capability: Option<&str>,
        eligible: impl Fn(&DecryptedConnection) -> bool,
    ) -> Result<ProjectMatch, String> {
        let query = query.trim();
        if item_id.is_some() && !query.is_empty() {
            return Err("Provide item or itemId, not both".to_owned());
        }
        if query.is_empty() && item_id.is_none()
            && let Some(connection) = self.active_connection_for_action(capability)
            && eligible(&connection)
        {
            return Ok(ProjectMatch::Selected(connection, SelectionReason::Session));
        }
        let mut connections = self
            .vault
            .list_decrypted_connections()
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|connection| {
                connection.stored.enabled
                    && connection_supports_action(connection, capability)
                    && eligible(connection)
            })
            .collect::<Vec<_>>();
        if let Some(item_id) = item_id {
            return match connections
                .iter()
                .position(|connection| connection.stored.id == item_id)
            {
                Some(index) => Ok(ProjectMatch::Selected(
                    connections.remove(index),
                    SelectionReason::Explicit,
                )),
                None => Ok(ProjectMatch::NotFound(project_candidates(&connections))),
            };
        }
        if query.is_empty() {
            return match connections.len() {
                0 => Ok(ProjectMatch::NoCompatibleProject),
                1 => Ok(ProjectMatch::Selected(connections.remove(0), SelectionReason::Unique)),
                _ => Ok(ProjectMatch::Ambiguous(project_candidates(&connections))),
            };
        }
        let candidates = project_candidates(&connections);
        let mut matches = filter_connections_by_query(connections, query);
        match matches.len() {
            0 => Ok(ProjectMatch::NotFound(candidates)),
            1 => Ok(ProjectMatch::Selected(matches.remove(0), SelectionReason::Explicit)),
            _ => Ok(ProjectMatch::Ambiguous(project_candidates(&matches))),
        }
    }

    fn resolve_action_project(
        &self,
        query: &str,
        item_id: Option<Uuid>,
        capability: Option<&str>,
    ) -> Result<DecryptedConnection, String> {
        self.resolve_action_project_with_reason(query, item_id, capability)
            .map(|(connection, _)| connection)
    }

    fn resolve_action_project_with_reason(
        &self,
        query: &str,
        item_id: Option<Uuid>,
        capability: Option<&str>,
    ) -> Result<(DecryptedConnection, SelectionReason), String> {
        let query = query.trim();
        let matched = self.match_action_project(query, item_id, capability)?;
        if (!query.is_empty() || item_id.is_some()) && !matches!(matched, ProjectMatch::Selected(..))
        {
            self.clear_active_item();
        }
        match matched {
            ProjectMatch::Selected(connection, reason) => {
                if !matches!(reason, SelectionReason::Session) {
                    self.bind_active_item(connection.stored.id);
                }
                Ok((connection, reason))
            }
            ProjectMatch::NoCompatibleProject => Err(match capability {
                Some(capability) => {
                    format!(
                        "No enabled and supported {} action-capable OpenKeylet project",
                        action_label(capability)
                    )
                }
                None => "No OpenKeylet project is available for Agent use".to_owned(),
            }),
            ProjectMatch::NotFound(_) if item_id.is_some() => {
                Err("No enabled and compatible OpenKeylet project matches itemId".to_owned())
            }
            ProjectMatch::NotFound(_) => Err(format!("No enabled OpenKeylet project found:{query}")),
            ProjectMatch::Ambiguous(candidates) => {
                let names = candidates
                    .iter()
                    .map(|item| item.name.as_str())
                    .collect::<Vec<_>>()
                    .join("、");
                if query.is_empty() {
                    Err(format!("Multiple OpenKeylet projects are available:{names}; specify item"))
                } else {
                    Err(format!("The project name matches multiple results:{names}; use a more complete name"))
                }
            }
        }
    }

    fn plan_native_http(
        &self,
        item: &str,
        item_id: Option<Uuid>,
        workflow: NativeWorkflow,
    ) -> Result<ActionResponse, String> {
        let matched = self.match_action_project_where(item, item_id, Some("http"), |connection| {
            native_service(connection, &workflow).is_some()
        })?;
        let ProjectMatch::Selected(mut connection, reason) = matched else {
            return Ok(matched.workflow_response());
        };
        let project = SelectedProject::new(&connection.stored, reason);
        let capability = native_service(&connection, &workflow).ok_or("Native service unavailable")?;
        if capability.status == crate::api_provider::ConfigurationStatus::Unsupported {
            return Ok(ActionResponse::Unavailable {
                code: UnavailableCode::UnsupportedOperation,
                project: Some(project),
                candidates: Vec::new(),
            });
        }
        if capability.status != crate::api_provider::ConfigurationStatus::Ready {
            let fields = if capability.missing_fields.is_empty() {
                vec!["authentication".to_owned()]
            } else {
                capability.missing_fields
            };
            return Ok(ActionResponse::NeedsInput {
                project,
                code: NeedInputCode::MissingConfiguration,
                missing_inputs: fields.into_iter().map(|name| MissingInput { name, source: "project" }).collect(),
            });
        }
        let prepared = match workflow.prepare(&connection) {
            Ok(prepared) => prepared,
            Err(issue) => return Ok(ActionResponse::NeedsInput {
                project,
                code: if issue.invalid { NeedInputCode::InvalidParameter } else { NeedInputCode::MissingParameter },
                missing_inputs: vec![MissingInput { name: issue.field.to_owned(), source: issue.source }],
            }),
        };
        protect_native_context(&mut connection);
        let target = describe_http_target(&connection, TargetSource::Saved, url::Url::parse(&prepared.request.url).ok());
        Ok(ActionResponse::Ready {
            project,
            target,
            next_call: native_next_call(connection.stored.id, &workflow),
            lookup: prepared.lookup,
        })
    }

    fn prepare_ssh_connection(
        &self,
        item: &str,
        host: Option<&str>,
        port: Option<u16>,
        username: Option<&str>,
    ) -> Result<DecryptedConnection, String> {
        let mut connection = self.resolve_item_for_action(item, Some("ssh"))?;
        self.vault.guard_agent_secret_use(connection.stored.id).map_err(|error| error.to_string())?;
        if let Some(jump) = connection.stored.ssh_jump_item_id { self.vault.guard_agent_secret_use(jump).map_err(|error| error.to_string())?; }
        if let Some(host) = host.map(str::trim).filter(|value| !value.is_empty()) {
            connection.stored.host = host.to_owned();
        }
        if let Some(port) = port {
            if port == 0 {
                return Err("SSH port must be greater than 0".to_owned());
            }
            connection.stored.port = port;
        } else if connection.stored.port == 0 {
            connection.stored.port = 22;
        }
        if let Some(username) = username.map(str::trim).filter(|value| !value.is_empty()) {
            connection
                .secrets
                .named_secrets
                .insert("username".to_owned(), username.to_owned());
        }
        if connection.stored.host.trim().is_empty() {
            return Err("The SSH project has no saved host; provide host in this call".to_owned());
        }
        if connection.secrets.get("username").is_none() {
            return Err(
                "The SSH project has no saved username; provide username in this call".to_owned(),
            );
        }
        Ok(connection)
    }

    fn resolve_secret_bindings(
        &self,
        connection: &mut DecryptedConnection,
        bindings: &HashMap<String, String>,
        item_hint: &str,
    ) -> Result<HashMap<String, String>, String> {
        self.vault.guard_agent_secret_use(connection.stored.id).map_err(|error| error.to_string())?;
        let mut replacements = HashMap::new();
        for (raw_alias, module_hint) in bindings {
            let alias = raw_alias.trim();
            if alias.is_empty() || alias.chars().any(char::is_control) {
                return Err("Invalid secret binding name".to_owned());
            }
            if replacements.contains_key(alias) {
                return Err(format!("Duplicate secret binding name:{alias}"));
            }
            let module = resolve_secret_module(connection, module_hint, item_hint)?;
            let (_, kind, mut value) = self
                .vault
                .agent_secret_value(connection.stored.id, &module)
                .map_err(|error| error.to_string())?;
            if kind == "totp" {
                value = current_totp(&value).map_err(|error| error.to_string())?;
            }
            let mut redaction_index = connection.secrets.named_secrets.0.len();
            let redaction_name = loop {
                let candidate = format!("__openkeylet_binding_{redaction_index}");
                if !connection.secrets.named_secrets.0.contains_key(&candidate) {
                    break candidate;
                }
                redaction_index += 1;
            };
            connection
                .secrets
                .named_secrets
                .insert(redaction_name, value.clone());
            replacements.insert(alias.to_owned(), value);
        }
        Ok(replacements)
    }

    fn prepare_http_secret_bindings(
        &self,
        connection: &mut DecryptedConnection,
        input: &mut HttpSendInput,
    ) -> Result<(), String> {
        let placeholders = collect_http_secret_placeholders(input)?;
        if placeholders.is_empty() {
            return Ok(());
        }
        let bindings = input
            .secret_bindings
            .iter()
            .filter(|(alias, _)| placeholders.contains(*alias))
            .map(|(alias, module)| (alias.clone(), module.clone()))
            .collect::<HashMap<_, _>>();
        let replacements = self.resolve_secret_bindings(connection, &bindings, &input.item)?;
        interpolate_http_input(input, &replacements)?;
        Ok(())
    }
}

fn project_candidates(connections: &[DecryptedConnection]) -> Vec<ProjectCandidate> {
    connections
        .iter()
        .map(|connection| (&connection.stored).into())
        .collect()
}

fn http_target_source(request_url: &str) -> TargetSource {
    if request_url.trim().is_empty() {
        TargetSource::Saved
    } else if url::Url::parse(request_url.trim()).is_ok() {
        TargetSource::Explicit
    } else {
        TargetSource::Relative
    }
}

fn http_target_is_visible(connection: &DecryptedConnection, source: TargetSource) -> bool {
    matches!(source, TargetSource::Explicit)
        || connection
            .stored
            .modules
            .iter()
            .filter(|module| {
                module.kind == "url"
                    || connection.stored.oci_bindings.as_ref()
                        .is_some_and(|bindings| bindings.region == Some(module.id))
            })
            .all(crate::model::ItemModule::agent_visible)
}

fn describe_http_target(
    connection: &DecryptedConnection,
    source: TargetSource,
    mut url: Option<url::Url>,
) -> HttpTarget {
    let url = if http_target_is_visible(connection, source) {
        url.take().map(|mut url| {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            crate::policy::redact(url.to_string(), &connection.stored, &connection.secrets)
        })
    } else {
        None
    };
    HttpTarget { source, url }
}

fn structured_http_data(response: &ApiResponse, connection: &DecryptedConnection) -> Option<Value> {
    let media_type = response.headers.get("content-type")?.split(';').next()?;
    let media_type = media_type.trim().to_ascii_lowercase();
    if response.truncated
        || response.saved_to.is_some()
        || !(media_type == "application/json" || media_type.ends_with("+json"))
    {
        return None;
    }
    let data = serde_json::from_str(&response.body).ok()?;
    let secrets = zeroize::Zeroizing::new(connection.secrets.non_empty_values());
    let candidates = zeroize::Zeroizing::new(crate::policy::redaction_candidates(&secrets));
    Some(redact_http_json(data, &candidates))
}

fn native_service(
    connection: &DecryptedConnection,
    workflow: &NativeWorkflow,
) -> Option<crate::api_provider::ServiceCapability> {
    crate::api_provider::describe_item(&connection.stored, &connection.secrets)
        .ok()
        .flatten()
        .filter(|service| service.provider == workflow.provider())
}

fn native_next_call(item_id: Uuid, workflow: &NativeWorkflow) -> NextCall {
    NextCall {
        tool: "http_send",
        arguments: json!({ "itemId": item_id, "workflow": workflow, "responseFormat": "structured" }),
    }
}

fn protect_native_context(connection: &mut DecryptedConnection) {
    let mut ids = Vec::new();
    if let Some(bindings) = &connection.stored.oci_bindings {
        ids.extend([bindings.region, bindings.tenancy_ocid, bindings.user_ocid, bindings.fingerprint]);
    }
    if let Some(bindings) = &connection.stored.aws_bindings {
        ids.extend([bindings.region, bindings.service_url]);
    }
    for id in ids.into_iter().flatten() {
        let module = connection.stored.modules.iter().find(|module| module.id == id);
        if module.is_some_and(|module| !module.agent_visible())
            && let Some(value) = crate::oci::binding_value(&connection.stored.modules, &connection.secrets, Some(id))
        {
            connection.secrets.named_secrets.insert(Uuid::new_v4().to_string(), value.to_owned());
        }
    }
}

fn redact_http_json(value: Value, candidates: &[String]) -> Value {
    let redact = |text| {
        candidates.iter().fold(text, |text: String, candidate| {
            text.replace(candidate, "[REDACTED]")
        })
    };
    match value {
        Value::String(text) => Value::String(redact(text)),
        Value::Array(values) => Value::Array(
            values.into_iter().map(|value| redact_http_json(value, candidates)).collect(),
        ),
        Value::Object(object) => Value::Object(
            object.into_iter()
                .map(|(name, value)| (redact(name), redact_http_json(value, candidates)))
                .collect(),
        ),
        other => other,
    }
}

fn connection_supports_action(connection: &DecryptedConnection, capability: Option<&str>) -> bool {
    if capability == Some("certificate") {
        return connection.stored.certificate_authority.is_some() || connection.stored.modules.iter().any(|module| module.kind == "privateKey");
    }
    if capability.is_some() && connection.stored.certificate_authority.is_some() { return false; }
    match capability {
        None => !connection
            .stored
            .capabilities(&connection.secrets)
            .is_empty(),
        Some(capability) => connection
            .stored
            .has_capability(&connection.secrets, capability),
    }
}

fn action_label(capability: &str) -> &str {
    match capability {
        "ssh" => "SSH",
        capability => capability,
    }
}

fn filter_connections_by_query(
    mut connections: Vec<DecryptedConnection>,
    query: &str,
) -> Vec<DecryptedConnection> {
    let normalized_query = query.trim().to_lowercase();
    let search_query = normalize_search_text(query);
    let has_literal_exact = connections
        .iter()
        .any(|item| item.stored.name.trim().to_lowercase() == normalized_query);
    let has_normalized_exact = !search_query.is_empty()
        && connections
            .iter()
            .any(|item| normalize_search_text(&item.stored.name) == search_query);
    let longest_embedded_name = if !has_literal_exact && !has_normalized_exact {
        connections
            .iter()
            .map(|item| normalize_search_text(&item.stored.name))
            .filter(|name| phrase_contains(&search_query, name))
            .map(|name| name.len())
            .max()
    } else {
        None
    };
    connections.retain(|item| {
        let name = item.stored.name.trim().to_lowercase();
        if has_literal_exact {
            name == normalized_query
        } else if has_normalized_exact {
            normalize_search_text(&item.stored.name) == search_query
        } else if let Some(longest) = longest_embedded_name {
            let normalized_name = normalize_search_text(&item.stored.name);
            normalized_name.len() == longest && phrase_contains(&search_query, &normalized_name)
        } else {
            let normalized_name = normalize_search_text(&item.stored.name);
            name.contains(&normalized_query)
                || phrase_contains(&search_query, &normalized_name)
                || phrase_contains(&normalized_name, &search_query)
        }
    });
    connections
}

fn normalize_search_text(value: &str) -> String {
    value
        .to_lowercase()
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn phrase_contains(haystack: &str, needle: &str) -> bool {
    !needle.is_empty() && format!(" {haystack} ").contains(&format!(" {needle} "))
}

fn module_aliases(name: &str) -> &'static [&'static str] {
    match name {
        "username" => &[
            "username",
            "user name",
            "account",
            "email",
            "Username",
            "Account",
        ],
        "password" => &["password", "passwd", "pwd", "Password"],
        "apiCredential" => &[
            "api credential",
            "api key",
            "access token",
            "bearer token",
            "token",
            "api credential",
        ],
        "privateKey" => &["private key", "ssh key", "identity file", "Private key"],
        "passphrase" => &[
            "passphrase",
            "key passphrase",
            "private key password",
            "Private key passphrase",
            "password",
        ],
        "totp" => &[
            "totp",
            "otp",
            "one time code",
            "verification code",
            "2fa code",
            "verification code",
        ],
        _ => &[],
    }
}

fn secret_module_match_score(name: &str, phrase: &str) -> Option<usize> {
    let phrase = normalize_search_text(phrase);
    let normalized_name = normalize_search_text(name);
    if phrase == normalized_name {
        return Some(10_000 + normalized_name.len());
    }
    std::iter::once(normalized_name)
        .chain(
            module_aliases(name)
                .iter()
                .map(|alias| normalize_search_text(alias)),
        )
        .filter(|alias| phrase_contains(&phrase, alias))
        .map(|alias| alias.len())
        .max()
}

fn module_hint_from_item(item_phrase: &str, project_name: &str) -> String {
    let phrase = format!(" {} ", normalize_search_text(item_phrase));
    let project = format!(" {} ", normalize_search_text(project_name));
    phrase.replace(&project, " ").trim().to_owned()
}

fn resolve_secret_module(
    connection: &DecryptedConnection,
    requested: &str,
    item_phrase: &str,
) -> Result<String, String> {
    let fields = connection
        .secrets
        .available_fields(connection.stored.secret.as_ref())
        .into_iter()
        .filter(|field| connection.secrets.get(&field.name).is_some())
        .collect::<Vec<_>>();
    match fields.as_slice() {
        [] => return Err("This project has no configured secret modules".to_owned()),
        [field] if requested.trim().is_empty() => return Ok(field.name.clone()),
        _ => {}
    }

    let hint = if requested.trim().is_empty() {
        module_hint_from_item(item_phrase, &connection.stored.name)
    } else {
        requested.trim().to_owned()
    };
    let scored = fields
        .iter()
        .filter_map(|field| {
            secret_module_match_score(&field.name, &hint).map(|score| (field.name.clone(), score))
        })
        .collect::<Vec<_>>();
    let best_score = scored.iter().map(|(_, score)| *score).max();
    let best = best_score.map_or_else(Vec::new, |best_score| {
        scored
            .into_iter()
            .filter(|(_, score)| *score == best_score)
            .map(|(name, _)| name)
            .collect::<Vec<_>>()
    });
    if let [name] = best.as_slice() {
        return Ok(name.clone());
    }
    let names = fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join("、");
    if requested.trim().is_empty() {
        Err(format!(
            "This project contains multiple secret modules; specify which module to use in the item statement, or specify module:{names}"
        ))
    } else {
        Err(format!(
            "Unable to uniquely match the secret module; available modules:{names}"
        ))
    }
}

// Keep existing automation templates usable after the KRU -> OpenKeylet rename.
fn next_secret_placeholder(input: &str) -> Option<(usize, usize)> {
    ["{{openkeylet:", "{{kru:"]
        .into_iter()
        .filter_map(|prefix| input.find(prefix).map(|start| (start, prefix.len())))
        .min_by_key(|(start, _)| *start)
}

fn interpolate_http_text(
    input: &str,
    replacements: &HashMap<String, String>,
    used: &mut HashSet<String>,
) -> Result<String, String> {
    let mut rest = input;
    let mut output = String::with_capacity(input.len());
    while let Some((start, prefix_len)) = next_secret_placeholder(rest) {
        output.push_str(&rest[..start]);
        let after_open = &rest[start + prefix_len..];
        let Some(end) = after_open.find("}}") else {
            return Err("Missing OpenKeylet secret placeholder }}".to_owned());
        };
        let alias = after_open[..end].trim();
        let value = replacements.get(alias).ok_or_else(|| {
            format!("OpenKeylet HTTP secret placeholder has no corresponding binding:{alias}")
        })?;
        output.push_str(value);
        used.insert(alias.to_owned());
        rest = &after_open[end + 2..];
    }
    output.push_str(rest);
    Ok(output)
}

fn collect_secret_placeholders_text(
    input: &str,
    placeholders: &mut HashSet<String>,
) -> Result<(), String> {
    let mut rest = input;
    while let Some((start, prefix_len)) = next_secret_placeholder(rest) {
        let after_open = &rest[start + prefix_len..];
        let Some(end) = after_open.find("}}") else {
            return Err("Missing OpenKeylet HTTP secret placeholder }}".to_owned());
        };
        let module = after_open[..end].trim();
        if module.is_empty() {
            return Err("OpenKeylet secret placeholder name cannot be empty".to_owned());
        }
        placeholders.insert(module.to_owned());
        rest = &after_open[end + 2..];
    }
    Ok(())
}

fn collect_secret_placeholders_value(
    value: &Value,
    placeholders: &mut HashSet<String>,
) -> Result<(), String> {
    match value {
        Value::String(text) => collect_secret_placeholders_text(text, placeholders)?,
        Value::Array(values) => {
            for value in values {
                collect_secret_placeholders_value(value, placeholders)?;
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_secret_placeholders_value(value, placeholders)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn collect_http_secret_placeholders(input: &HttpSendInput) -> Result<HashSet<String>, String> {
    let mut placeholders = HashSet::new();
    collect_secret_placeholders_text(&input.url, &mut placeholders)?;
    for value in input.headers.values() {
        collect_secret_placeholders_text(value, &mut placeholders)?;
    }
    for value in input.query.values() {
        collect_secret_placeholders_value(value, &mut placeholders)?;
    }
    if let Some(body) = input.body.as_ref() {
        collect_secret_placeholders_value(body, &mut placeholders)?;
    }
    for value in input.form.values() {
        collect_secret_placeholders_value(value, &mut placeholders)?;
    }
    Ok(placeholders)
}

fn include_http_secret_bindings(input: &mut HttpSendInput) -> Result<(), String> {
    for module in collect_http_secret_placeholders(input)? {
        input.secret_bindings.entry(module.clone()).or_insert(module);
    }
    Ok(())
}

fn interpolate_http_value(
    value: &mut Value,
    replacements: &HashMap<String, String>,
    used: &mut HashSet<String>,
) -> Result<(), String> {
    match value {
        Value::String(text) => *text = interpolate_http_text(text, replacements, used)?,
        Value::Array(values) => {
            for value in values {
                interpolate_http_value(value, replacements, used)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                interpolate_http_value(value, replacements, used)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn interpolate_http_input(
    input: &mut HttpSendInput,
    replacements: &HashMap<String, String>,
) -> Result<HashSet<String>, String> {
    let mut used = HashSet::new();
    input.url = interpolate_http_text(&input.url, replacements, &mut used)?;
    for value in input.headers.values_mut() {
        *value = interpolate_http_text(value, replacements, &mut used)?;
    }
    for value in input.query.values_mut() {
        interpolate_http_value(value, replacements, &mut used)?;
    }
    if let Some(body) = input.body.as_mut() {
        interpolate_http_value(body, replacements, &mut used)?;
    }
    for value in input.form.values_mut() {
        interpolate_http_value(value, replacements, &mut used)?;
    }
    Ok(used)
}

fn tool_success<T: Serialize>(value: T) -> CallToolResult {
    match serde_json::to_value(value) {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            let mut result = CallToolResult::structured(value);
            result.content = vec![ContentBlock::text(text)];
            result
        }
        Err(_) => tool_error("OpenKeylet could not serialize the tool result."),
    }
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn into_tool_result<T: Serialize>(result: Result<T, String>) -> CallToolResult {
    match result {
        Ok(value) => tool_success(value),
        Err(message) => tool_error(message),
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ItemsSearchInput {
    #[serde(default)]
    #[schemars(
        description = "Optional project query. It may be an exact name, a partial name, or a natural-language phrase containing the project name. Exact case-insensitive matches win."
    )]
    query: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ItemsSearchOutput {
    items: Vec<ItemOutput>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ItemOutput {
    #[serde(skip_serializing_if = "Option::is_none")]
    api_service: Option<crate::api_provider::ServiceCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    certificate_authority: Option<crate::model::CertificateAuthorityBinding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aws_bindings: Option<crate::model::AwsModuleBindings>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oci_bindings: Option<crate::model::OciModuleBindings>,
    ssh_jump_item_id: Option<String>,
    ssh_jump_unresolved: bool,
    id: String,
    revision: u64,
    name: String,
    modules: Vec<ItemModuleOutput>,
    actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(description = "Non-secret OCI request context. tenancyOcid is the root compartment OCID; region determines the allowed regional service endpoints. Only context fields whose bound modules are agent-visible are included.")]
    oci_context: Option<OciContext>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct OciContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenancy_ocid: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ItemModuleOutput {
    id: String,
    kind: String,
    name: String,
    configured: bool,
    agent_visible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    alternate_values: Option<Vec<String>>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct CredentialFillOutput {
    target: String,
    module: String,
    submitted: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct OkOutput {
    ok: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SshRunInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name, or an unambiguous natural-language phrase containing it. Omit it after this MCP session selected a compatible project, or when exactly one enabled project stores a password or private key."
    )]
    item: String,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH host override, including another saved address. Omit it to use the default host. Other addresses are never tried automatically."
    )]
    host: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH port override. Omit it to use the saved port, or port 22 when the project has no port."
    )]
    port: Option<u16>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH username override. Omit it to use the encrypted username saved in the OpenKeylet project."
    )]
    username: Option<String>,
    #[schemars(
        description = "One command to execute. A hidden module may be used directly as {{openkeylet:password}} or {{openkeylet:module name}} without surrounding quotes; OpenKeylet inserts a shell-quoted value."
    )]
    command: String,
    #[serde(default)]
    #[schemars(
        description = "Optional text written directly to the remote command's standard input. It may contain {{openkeylet:module name}} placeholders, which OpenKeylet substitutes and redacts locally."
    )]
    stdin: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional remote working directory. A hidden module may be used directly as {{openkeylet:module name}}."
    )]
    cwd: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional execution timeout in seconds. Omit or use 0 for no command deadline."
    )]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional environment-variable mapping from variable name to any secret module in item. OpenKeylet injects each value into the remote command environment and redacts it from output without returning it to the caller. The command can reference variables such as $OPENKEYLET_PASSWORD."
    )]
    secret_env: HashMap<String, String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SshUploadInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name, or an unambiguous natural-language phrase containing it. Omit it after this MCP session selected a compatible project, or when exactly one enabled project stores a password or private key."
    )]
    item: String,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH host override, including another saved address. Omit it to use the default host. Other addresses are never tried automatically."
    )]
    host: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH port override. Omit it to use the saved port, or port 22 when the project has no port."
    )]
    port: Option<u16>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH username override. Omit it to use the encrypted username saved in the OpenKeylet project."
    )]
    username: Option<String>,
    #[schemars(
        description = "Local file or directory path on the machine running OpenKeylet. Absolute and ~/ paths are accepted; relative paths resolve from the OpenKeylet MCP process working directory. Directories are transferred recursively."
    )]
    local_path: String,
    #[serde(default)]
    #[schemars(
        description = "Optional remote file or directory path. Omit it to upload to the SSH login directory using the local source name."
    )]
    remote_path: String,
    #[serde(default = "default_true")]
    #[schemars(description = "Replace an existing destination file. Defaults to true.")]
    overwrite: bool,
    #[serde(default)]
    #[schemars(
        description = "Optional transfer timeout in seconds. Omit or use 0 for no total deadline."
    )]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SshDownloadInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name, or an unambiguous natural-language phrase containing it. Omit it after this MCP session selected a compatible project, or when exactly one enabled project stores a password or private key."
    )]
    item: String,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH host override, including another saved address. Omit it to use the default host. Other addresses are never tried automatically."
    )]
    host: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH port override. Omit it to use the saved port, or port 22 when the project has no port."
    )]
    port: Option<u16>,
    #[serde(default)]
    #[schemars(
        description = "Optional SSH username override. Omit it to use the encrypted username saved in the OpenKeylet project."
    )]
    username: Option<String>,
    #[schemars(
        description = "Remote file or directory path on the SSH host. Directories are transferred recursively."
    )]
    remote_path: String,
    #[serde(default)]
    #[schemars(
        description = "Optional local file or directory path. Omit it to save in the OpenKeylet MCP process working directory using the remote source name."
    )]
    local_path: String,
    #[serde(default = "default_true")]
    #[schemars(description = "Replace an existing destination file. Defaults to true.")]
    overwrite: bool,
    #[serde(default)]
    #[schemars(
        description = "Optional transfer timeout in seconds. Omit or use 0 for no total deadline."
    )]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HttpSendInput {
    #[schemars(description = "Typed read-only native operation from action_capabilities. Requires itemId and supplies its own URL, method, query and authentication.")]
    workflow: Option<NativeWorkflow>,
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name, or an unambiguous natural-language phrase containing it. Omit it after this MCP session has selected a compatible project, or when exactly one compatible project exists."
    )]
    item: String,
    #[schemars(description = "Stable project UUID for exact selection. Omit item when providing itemId.")]
    item_id: Option<Uuid>,
    #[serde(default)]
    #[schemars(description = "Use structured to include the actual project, parsed complete JSON, and HTTP error details. Defaults to legacy.")]
    response_format: HttpResponseFormat,
    #[serde(default)]
    #[schemars(
        description = "Optional absolute URL or path relative to the project's saved service URL. OCI relative paths use the default Compute base URL; use an absolute OCI regional URL for other services. OCI requires HTTPS on port 443 and a regional oraclecloud.com endpoint matching ociContext.region. Omit to request the saved service URL itself. An item without a saved service URL requires an absolute URL."
    )]
    url: String,
    #[serde(default = "default_http_method")]
    #[schemars(description = "HTTP method. Defaults to GET. For OCI the agent chooses the API operation and method; OpenKeylet signs the request, and OCI IAM enforces resource permissions. CONNECT and TRACE transport methods are not supported for OCI.")]
    method: String,
    #[serde(default)]
    #[schemars(description = "Query parameters appended to the URL.")]
    query: HashMap<String, Value>,
    #[serde(default)]
    #[schemars(description = "Non-authentication request headers, such as Content-Type and opc-request-id. OCI signing and transport headers are generated locally and cannot be overridden.")]
    headers: HashMap<String, String>,
    #[serde(default)]
    #[schemars(
        description = "Optional request body. Objects, arrays, numbers, and booleans are encoded as JSON; a string is sent directly as text. For OCI the final serialized body, including form/files or bodyBase64 when used instead, is buffered for signing and limited to 64 MiB."
    )]
    body: Option<Value>,
    #[serde(default)]
    #[schemars(description = "Optional URL-encoded form fields. Array values repeat a field.")]
    form: HashMap<String, Value>,
    #[serde(default)]
    #[schemars(description = "Optional multipart file uploads from local paths.")]
    files: Vec<crate::executor::ApiUploadFile>,
    #[serde(default)]
    #[schemars(description = "Optional Base64-encoded raw request body.")]
    body_base64: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional aliases from a placeholder name to a secret module. Usually omit this map and write the module directly, such as {{openkeylet:password}} or {{openkeylet:service token}}. Use secretBindings only when the placeholder should have a different short name. OpenKeylet substitutes and redacts values locally."
    )]
    secret_bindings: HashMap<String, String>,
    #[serde(default)]
    #[schemars(
        description = "Optional request timeout in seconds. Omit or use 0 for no total deadline."
    )]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional response-body limit in bytes. For OCI, omit or use 0 for the 2 MiB default; a positive value also bounds saveResponseTo. For other HTTP projects, omit or use 0 for no in-context limit; saveResponseTo streams without this limit."
    )]
    max_response_bytes: Option<usize>,
    #[serde(default)]
    #[schemars(
        description = "Optional local file path for the complete response. OCI buffers and redacts within maxResponseBytes before atomic saving, and rejects oversized responses without a partial file. Other HTTP projects stream without the in-context limit. Absolute and ~/ paths are accepted; relative paths resolve from the OpenKeylet MCP process working directory. Parent directories are created automatically."
    )]
    save_response_to: Option<String>,
    #[serde(default = "default_true")]
    #[schemars(
        description = "Replace an existing response file. Defaults to true and only applies with saveResponseTo."
    )]
    overwrite_response_file: bool,
}

fn default_http_method() -> String {
    "GET".to_owned()
}

fn default_true() -> bool {
    true
}

fn default_shell_secret_literal(value: &str) -> String {
    #[cfg(windows)]
    {
        format!("'{}'", value.replace('\'', "''"))
    }
    #[cfg(not(windows))]
    {
        crate::executor::shell_quote(value)
    }
}

fn default_terminal_program() -> String {
    #[cfg(windows)]
    {
        "powershell.exe".to_owned()
    }
    #[cfg(not(windows))]
    {
        std::env::var("SHELL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "/bin/sh".to_owned())
    }
}

fn submitted_terminal_line(command: &str) -> String {
    if command.ends_with('\n') || command.ends_with('\r') {
        return command.to_owned();
    }
    #[cfg(windows)]
    let newline = "\r\n";
    #[cfg(not(windows))]
    let newline = "\n";
    format!("{command}{newline}")
}

fn transfer_file_name(path: &str, label: &str) -> Result<String, String> {
    let trimmed = path.trim().trim_end_matches(['/', '\\']);
    let name = trimmed
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty() && !matches!(*name, "." | ".."))
        .ok_or_else(|| format!("Could not use {label} to derive a filename"))?;
    Ok(name.to_owned())
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CredentialFillInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name, or an unambiguous natural-language phrase containing it. Omit it after this MCP session or terminal has selected a compatible project, or when exactly one project supports credential filling."
    )]
    item: String,
    #[schemars(description = "Stable project UUID for exact selection. Omit item when providing itemId.")]
    item_id: Option<Uuid>,
    #[serde(default)]
    #[schemars(
        description = "Optional module name or common phrase such as password, API key, private key, or verification code. Omit it when the project has one configured secret, or mention the module in the item phrase."
    )]
    module: String,
    #[serde(default)]
    #[schemars(
        description = "Optional write target: browser, desktop, or terminal. Omit it to use the current running OpenKeylet terminal when one exists, otherwise a connected browser extension, otherwise the real operating-system foreground control."
    )]
    target: String,
    #[serde(default)]
    #[schemars(
        description = "Optional terminal session ID. Omit it to use the current OpenKeylet terminal; provide it only to switch among multiple terminal sessions."
    )]
    session_id: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Submit after filling. For terminal and desktop targets this presses Enter; for browser targets it submits the focused field's form when one exists. Defaults to false."
    )]
    submit: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalStartInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name or natural-language phrase. Pass it once to bind this terminal session, including when the first command has no secret; later terminal_write calls can then omit item. It is required for placeholders or secretEnv only when no unique or bound project can be inferred."
    )]
    item: String,
    #[serde(default)]
    #[schemars(
        description = "Optional program name or path selected by the agent. Omit it to start PowerShell on Windows, or $SHELL with a /bin/sh fallback on macOS and Linux. Native executables start directly; Windows .cmd and .bat scripts use cmd.exe."
    )]
    program: String,
    #[serde(default)]
    #[schemars(
        description = "Arguments passed as individual argv values. A hidden module may be used directly as {{openkeylet:password}} or {{openkeylet:module name}}."
    )]
    args: Vec<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional working directory. Absolute and ~/ paths are accepted; relative paths resolve from the OpenKeylet MCP process working directory. A hidden module may be used directly as {{openkeylet:module name}}."
    )]
    cwd: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional first line submitted immediately after the terminal starts. When program is omitted, this runs as a command in the platform's default shell. A hidden module may be used directly as {{openkeylet:password}} or {{openkeylet:module name}} without surrounding quotes; OpenKeylet handles shell quoting."
    )]
    command: String,
    #[serde(default)]
    #[schemars(
        description = "Optional environment-variable mapping from variable name to OpenKeylet module name or common module phrase. OpenKeylet injects each value into the child process and redacts it from terminal output without returning it to the caller."
    )]
    secret_env: HashMap<String, String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalRunInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name or natural-language phrase. Omit it after this MCP session selected a compatible project, or when the command does not use a hidden module."
    )]
    item: String,
    #[schemars(
        description = "Shell command to run to completion. A hidden module may be used directly as {{openkeylet:password}} or {{openkeylet:module name}} without surrounding quotes; OpenKeylet inserts a shell-quoted value."
    )]
    command: String,
    #[serde(default)]
    #[schemars(
        description = "Optional text written directly to the command's standard input. It may contain {{openkeylet:module name}} placeholders, which OpenKeylet substitutes and redacts locally."
    )]
    stdin: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional working directory. Absolute, ~/ and MCP-session-relative paths are accepted. A hidden module may be used directly as {{openkeylet:module name}}."
    )]
    cwd: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Optional execution timeout in seconds. Omit or use 0 for no command deadline."
    )]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional environment-variable mapping from variable name to a secret module. OpenKeylet injects and redacts each value locally."
    )]
    secret_env: HashMap<String, String>,
    #[serde(default)]
    #[schemars(
        description = "Optional redacted .env import template copied from OpenKeylet. Each line binds one environment variable to exact project and module IDs. Values are resolved locally and redacted from output. Use this independently of item, secretEnv, and inline placeholders."
    )]
    env_template: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalSessionInput {
    #[serde(default)]
    #[schemars(
        description = "Optional session ID returned by terminal_start. Omit it to use the most recently started or explicitly used terminal in this MCP session."
    )]
    session_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalReadInput {
    #[serde(default)]
    #[schemars(
        description = "Optional session ID returned by terminal_start. Omit it to read the current terminal for this MCP session."
    )]
    session_id: String,
    #[serde(default)]
    #[schemars(
        description = "Optional maximum seconds to wait. Omit it for an unlimited wait when until is provided, or for an immediate read when until is absent."
    )]
    wait_seconds: Option<u64>,
    #[serde(default)]
    #[schemars(
        description = "Optional output substring to wait for. OpenKeylet accumulates output until this text appears or the process exits. Omit waitSeconds for no OpenKeylet deadline, or set it to bound the wait."
    )]
    until: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TerminalWriteInput {
    #[serde(default)]
    #[schemars(
        description = "Optional OpenKeylet project name or natural-language phrase used when text contains a {{openkeylet:module}} placeholder. Omit it when terminal_start or an earlier write already bound this session, or when exactly one enabled project contains usable secrets."
    )]
    item: String,
    #[serde(default)]
    #[schemars(
        description = "Optional session ID returned by terminal_start. Omit it to write to the current terminal for this MCP session."
    )]
    session_id: String,
    #[schemars(
        description = "Interactive terminal input. A hidden module may be inserted as raw input with {{openkeylet:password}} or {{openkeylet:module name}}. OpenKeylet redacts an echoed value from later terminal output."
    )]
    text: String,
    #[serde(default)]
    #[schemars(
        description = "Press Enter after text. OpenKeylet uses the correct newline for the current platform and avoids adding a second newline when text already ends with one. Defaults to false."
    )]
    submit: bool,
}

// Suppress serde diagnostics that could echo malformed credential input.
#[derive(JsonSchema)]
#[schemars(transparent)]
struct WriteRequest<T>(T);
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for WriteRequest<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        serde_json::from_value(value).map(Self).map_err(|_|serde::de::Error::custom("Invalid write request: check field names and types; no changes saved"))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CertificateIssueInput {
    item: String,
    #[schemars(
        description = "A new UUID for a new issuance. Reuse the same UUID and identical request/TTL only to recover a committed result after a transport retry."
    )]
    request_id: String,
    request: crate::certificates::CertificateRequest,
    ttl_seconds: u64,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CertificateInspectInput {
    certificate: String,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CertificateCheckInput {
    item: String,
    key_module_id: Option<String>,
    public_or_certificate: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ApiServiceCapabilitiesInput {
    #[serde(default)]
    item: String,
    url: Option<String>,
}

#[tool_router(router = tool_router)]
impl VaultMcp {
    #[tool(
        name = "action_capabilities",
        description = "Preview the next step without executing it or changing session selection. Use selectProject for selection alone, httpRequest for generic HTTP, or nativeApi for a provider-specific read and pagination. Reports the exact project, eligible candidates or missing inputs.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<ActionResponse>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn action_capabilities(
        &self,
        Parameters(input): Parameters<ActionCapabilitiesInput>,
    ) -> CallToolResult {
        let result = (|| {
            match input {
                ActionCapabilitiesInput::SelectProject { capability, item, item_id } => self
                    .match_action_project(&item, item_id, Some(capability.as_str()))
                    .map(ProjectMatch::workflow_response),
                ActionCapabilitiesInput::NativeApi { item, item_id, workflow } =>
                    self.plan_native_http(&item, item_id, workflow),
                ActionCapabilitiesInput::HttpRequest { item, item_id, url, method, query, body } => {
                    let project = self.match_action_project(&item, item_id, Some("http"))?;
                    let ProjectMatch::Selected(mut connection, reason) = project else {
                        return Ok(project.workflow_response());
                    };
                    let project = SelectedProject::new(&connection.stored, reason);
                    let method = method.unwrap_or_else(default_http_method);
                    let source = http_target_source(&url);
                    let mut arguments = json!({
                        "itemId": connection.stored.id,
                        "method": method,
                        "responseFormat": "structured",
                    });
                    if !url.is_empty() {
                        arguments["url"] = json!(url);
                    }
                    if !query.is_empty() {
                        arguments["query"] = json!(query);
                    }
                    if let Some(body) = body {
                        arguments["body"] = body;
                    }
                    let mut draft: HttpSendInput = serde_json::from_value(arguments.clone())
                        .map_err(|_| "Unable to prepare HTTP workflow arguments".to_owned())?;
                    if include_http_secret_bindings(&mut draft)
                        .and_then(|_| self.prepare_http_secret_bindings(&mut connection, &mut draft))
                        .is_err()
                    {
                        return Ok(ActionResponse::NeedsInput {
                            project,
                            code: NeedInputCode::MissingSecretBinding,
                            missing_inputs: vec![MissingInput { name: "secretBindings".to_owned(), source: "project" }],
                        });
                    }
                    let target = match preview_api_request(&connection, &draft.url, &draft.method, &draft.query) {
                        Ok(target) => target,
                        Err(issue) => {
                            let (code, name, source) = match issue {
                                ApiPreviewIssue::MissingUrl => (NeedInputCode::MissingRequestUrl, "url", "caller"),
                                ApiPreviewIssue::InvalidUrl => (NeedInputCode::InvalidRequestUrl, "url", "caller"),
                                ApiPreviewIssue::InvalidMethod => (NeedInputCode::InvalidHttpMethod, "method", "caller"),
                                ApiPreviewIssue::MissingAuthentication => (NeedInputCode::MissingConfiguration, "authentication", "project"),
                                ApiPreviewIssue::UnsupportedTarget => (NeedInputCode::UnsupportedTarget, "url", "caller"),
                            };
                            return Ok(ActionResponse::NeedsInput {
                                project,
                                code,
                                missing_inputs: vec![MissingInput { name: name.to_owned(), source }],
                            });
                        }
                    };
                    Ok(ActionResponse::Ready {
                        project,
                        target: describe_http_target(&connection, source, Some(target)),
                        next_call: NextCall { tool: "http_send", arguments },
                        lookup: None,
                    })
                }
            }
        })();
        into_tool_result(result)
    }

    #[tool(name = "api_service_capabilities", description = "Identify the concrete API service and supported authentication from a URL, or inspect a selected item's native configuration. Returns required role names, readiness, official documentation and protocol/pagination guidance without credentials. URL-only discovery does not authorize sending stored secrets or claim configuration is ready. Supports AWS standard SigV4 endpoint subset, existing OCI RSA signing, GitHub REST, Cloudflare control plane, DigitalOcean control plane and Hetzner Cloud. S3, OAuth token acquisition and other vendor signers are not implied. Call http_send with the documented operation; OpenKeylet signs/authenticates locally but does not choose operations or enumerate pages.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::api_provider::ServiceCapability>(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn api_service_capabilities(
        &self,
        Parameters(input): Parameters<ApiServiceCapabilitiesInput>,
    ) -> CallToolResult {
        let result: Result<crate::api_provider::ServiceCapability, String> = (|| {
            if let Some(url) = input.url {
                if !input.item.trim().is_empty() {
                    return Err("Choose item or url, not both".into());
                }
                return crate::api_provider::describe_url(&url).map_err(|error| error.to_string());
            }
            let connection = self.resolve_item_for_action(&input.item, Some("http"))?;
            self.vault
                .guard_agent_secret_use(connection.stored.id)
                .map_err(|error| error.to_string())?;
            crate::api_provider::describe_item(&connection.stored, &connection.secrets).map_err(|error| error.to_string())?.ok_or_else(|| "No recognized native API service; this project uses generic HTTP authentication".into())
        })();
        into_tool_result(result)
    }
    #[tool(name = "certificate_capabilities", description = "Read owner-controlled local CA registration and Agent signing grants, permitted certificate kinds and maximum TTL. Signing is off by default and independent of folder write access. Reports fingerprints/status only, never CA private material. Agent cannot change grants or CA registration.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::vault::CertificateCapabilities>(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn certificate_capabilities(&self) -> CallToolResult {
        into_tool_result(
            self.vault
                .certificate_capabilities()
                .map_err(|error| error.to_string()),
        )
    }
    #[tool(name = "certificate_inspect", description = "Inspect a public SSH or PEM TLS certificate: identities, purpose, issuer/key fingerprints, serial and validity. Inspection alone does not establish trust. Supply public certificate text only, never a private key.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::certificates::CertificateSummary>(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn certificate_inspect(
        &self,
        Parameters(input): Parameters<CertificateInspectInput>,
    ) -> CallToolResult {
        into_tool_result(
            crate::certificates::inspect_certificate(&input.certificate)
                .map_err(|error| error.to_string()),
        )
    }
    #[tool(name = "certificate_verify", description = "Verify a public SSH or TLS certificate against explicitly supplied trusted public CA certificates/keys and the exact expected identity and purpose. Checks signature, trust, time, identity and purpose; reports revocationChecked=false. Do not infer trust from an unverified certificate or claim revocation was checked. No network requests.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::certificates::VerificationReport>(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn certificate_verify(
        &self,
        Parameters(input): Parameters<crate::certificates::VerifyRequest>,
    ) -> CallToolResult {
        into_tool_result(
            crate::certificates::verify_certificate(&input, chrono::Utc::now().timestamp())
                .map_err(|error| error.to_string()),
        )
    }
    #[tool(name = "certificate_check_key", description = "Check whether a saved private key matches a supplied public key or certificate, without exposing private material. Choose the project and optional stable private-key module ID. Returns algorithm, public fingerprint and matches only.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::certificates::KeyMatchReport>(), annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn certificate_check_key(
        &self,
        Parameters(input): Parameters<CertificateCheckInput>,
    ) -> CallToolResult {
        let result = (|| {
            let item = self.resolve_item_for_action(&input.item, Some("certificate"))?;
            let module = input
                .key_module_id
                .map(|id| Uuid::parse_str(&id).map_err(|_| "Invalid module UUID".to_owned()))
                .transpose()?;
            self.vault
                .check_item_key(item.stored.id, module, &input.public_or_certificate)
                .map_err(|error| error.to_string())
        })();
        into_tool_result(result)
    }
    #[tool(name = "certificate_issue", description = "Issue an SSH user/host or TLS server/client leaf using an owner-registered CA and current explicit Agent signing grant. First read certificate_capabilities. Submit only a public SSH key plus concrete principals/keyId, or a signed CSR plus exact names, and ttlSeconds within the grant. Never submit or request the CA private key. Use a UUID requestId; identical retries recover the committed certificate, changed payload/TTL or issuer conflicts. Rechecks current authorization on replay; revocation is immediate. Cannot create CAs, expand grants or change trust. Returns public certificate/chain. SSH user certificates can be saved as sshCertificate modules beside the matching privateKey and sshAuthType=certificate. Host certificates require separate user-confirmed host CA trust; never auto-trust a returned signer.", output_schema = rmcp::handler::server::tool::schema_for_type::<crate::certificates::IssuedCertificate>(), annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn certificate_issue(
        &self,
        Parameters(input): Parameters<CertificateIssueInput>,
    ) -> CallToolResult {
        let result = (|| {
            let item = self.resolve_item_for_action(&input.item, Some("certificate"))?;
            let request_id = Uuid::parse_str(&input.request_id)
                .map_err(|_| "Invalid request UUID".to_owned())?;
            self.vault
                .issue_certificate(
                    crate::vault::CertificateActor::Agent,
                    item.stored.id,
                    request_id,
                    input.request,
                    input.ttl_seconds,
                )
                .map_err(|error| error.to_string())
        })();
        into_tool_result(result)
    }
    #[tool(name = "vault_write_capabilities", description = "Read MCP vault write permissions, writable folders, supported fields and authentication modes. Writing is disabled by default. Does not reveal secrets or expand project query permissions.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<crate::vault_write::WriteCapabilities>(),
        annotations(read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false))]
    async fn vault_write_capabilities(&self) -> CallToolResult {
        into_tool_result(self.vault.write_capabilities().map_err(|_| "Could not read write permissions".to_owned()))
    }

    #[tool(name = "items_create", description = "Create a project in an authorized folder. Call vault_write_capabilities for fields. Requires name, itemType, folder and modules. Optional sshJumpItemId selects an existing enabled direct SSH project; trust records are local and cannot be written here. SSH needs host, username, password/privateKey and sshAuthType; API needs url, httpAuthType and credentials. Module value is the default address for host/url; alternateValues lists other addresses. Module values are strings except integer port. customSecret stores extra named values only. New secrets are hidden. Encrypted SSH keys require passphrase in this request. AWS and OCI native signing use awsBindings or ociBindings with stable module IDs. Create ordinary modules first, then bind returned IDs in an update; use vault_write_capabilities for role names. Returns IDs and revision only; never repeat secrets in chat.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<crate::vault_write::WriteOutput>(),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = false))]
    async fn items_create(&self, Parameters(WriteRequest(input)): Parameters<WriteRequest<crate::vault_write::CreateInput>>) -> CallToolResult {
        into_tool_result(self.vault.agent_create(input).map_err(|e|e.to_string()))
    }

    #[tool(name = "items_update", description = "Patch a project by id and expectedRevision. Update modules by stable id; omit id and provide kind/name/value to add. Omitted fields remain unchanged. Targets and authentication can change. For host/url, value is the default and alternateValues lists other addresses; omit the list to preserve it or use [] to clear it. Switch defaults by submitting both fields. sshJumpItemId selects an existing direct SSH project; null clears it and omission preserves it. Host trust cannot be changed. Moves require both folders authorized. Cannot change visibility or delete. AWS and OCI credentials and stable signing-module bindings may be updated under the same folder/revision rules. CA registration and signing grants remain owner-only. A conflict requires re-query, never replay automatically. Encrypted replacement SSH keys require passphrase; unencrypted replacements clear old passphrase. No secret values returned.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<crate::vault_write::WriteOutput>(),
        annotations(read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = false))]
    async fn items_update(&self, Parameters(WriteRequest(input)): Parameters<WriteRequest<crate::vault_write::UpdateInput>>) -> CallToolResult {
        into_tool_result(self.vault.agent_update(input).map_err(|e|e.to_string()))
    }

    #[tool(
        name = "items_search",
        description = "Discover or select enabled OpenKeylet projects, modules, and available actions. A query that finds one project makes it the current project for this MCP session, so later compatible actions can omit item. When the user already named a project and the intended action is clear, call that action directly instead. Query may be the project name or the user's natural-language phrase containing it. Exact case-insensitive name matches win. Only modules explicitly marked agentVisible include value. Recognized API projects include apiService protocol/readiness/documentation metadata and AWS bindings expose IDs only. Call api_service_capabilities for detailed service guidance. OCI projects also return non-secret ociContext with region and tenancyOcid for constructing API requests; hidden module values are never returned; a signing key is returned only when the owner explicitly enables its module visibility.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<ItemsSearchOutput>(),
        annotations(title = "Find OpenKeylet projects", read_only_hint = true, destructive_hint = false, idempotent_hint = true, open_world_hint = false)
    )]
    async fn items_search(
        &self,
        Parameters(input): Parameters<ItemsSearchInput>,
    ) -> CallToolResult {
        into_tool_result(self.search_items(input))
    }

    #[tool(
        name = "credential_fill",
        description = "Use one stored module without returning hidden plaintext. Omit item when this MCP session or terminal already selected a compatible project, or when exactly one enabled project supports filling; otherwise pass its name or a natural phrase containing it. OpenKeylet infers a sole secret module and understands module names or common phrases such as password, API key, private key, and verification code in either module or item. Omit target to use the current running OpenKeylet terminal, otherwise try a connected browser extension and immediately fall back to the real operating-system foreground control; common target aliases are accepted. sessionId is only needed to switch among multiple terminals. Set submit=true to complete the focused form or press Enter in the same call. Call items_search only when discovery or genuine ambiguity remains.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<CredentialFillOutput>(),
        annotations(title = "Fill a credential", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true)
    )]
    async fn credential_fill(
        &self,
        Parameters(input): Parameters<CredentialFillInput>,
    ) -> CallToolResult {
        let result: Result<CredentialFillOutput, String> = async {
            let requested_target = normalize_search_text(&input.target);
            let automatic_target = matches!(requested_target.as_str(), "" | "auto" | "default");
            let terminal_target = matches!(
                requested_target.as_str(),
                "terminal" | "term" | "pty" | "shell"
            );
            let terminal_session_id = if automatic_target || terminal_target {
                if let Some(session_id) = input.session_id.as_deref() {
                    Some(self.resolve_terminal_session_id(session_id, true)?)
                } else {
                    match self.resolve_terminal_session_id("", true) {
                        Ok(session_id) => Some(session_id),
                        Err(_) if automatic_target => None,
                        Err(error) => return Err(error),
                    }
                }
            } else {
                None
            };
            let connection = if input.item.trim().is_empty() && input.item_id.is_none() {
                if let Some(session_id) = terminal_session_id {
                    self.resolve_terminal_item(session_id, "")?
                } else {
                    self.resolve_item_for_action("", Some("fill"))?
                }
            } else {
                self.resolve_action_project(&input.item, input.item_id, Some("fill"))?
            };
            let item_id = connection.stored.id;
            let target = match requested_target.as_str() {
                "" | "auto" | "default" if terminal_session_id.is_some() => "terminal".to_owned(),
                "" | "auto" | "default" => {
                    if self.browser.fill_configured() {
                        "browser".to_owned()
                    } else {
                        "desktop".to_owned()
                    }
                }
                "browser" | "web" | "chromium" => "browser".to_owned(),
                "desktop" | "foreground" | "system" | "os" => {
                    "desktop".to_owned()
                }
                "terminal" | "term" | "pty" | "shell" => "terminal".to_owned(),
                _ => {
                    return Err(
                        "target can be browser, desktop, terminal, or a common synonym, and may also be omitted directly"
                            .to_owned(),
                    );
                }
            };
            let module = resolve_secret_module(&connection, &input.module, &input.item)?;
            let submit = input.submit;
            let action_target = if automatic_target { "auto" } else { &target };
            let actual_target = self
                .track(
                    item_id,
                    format!("to {action_target} fill in module {module}"),
                    async {
                        let (_, kind, mut value) = self
                            .vault
                            .agent_secret_value(item_id, &module)
                            .map_err(|error| anyhow::anyhow!(error))?;
                        if kind == "totp" {
                            value = current_totp(&value)?;
                        }
                        match target.as_str() {
                            "browser" => {
                                let browser_failure =
                                    match self.browser.fill_value(value.clone(), submit).await {
                                        Ok(result) if result.status == "ok" => {
                                            return Ok("browser".to_owned());
                                        }
                                        Ok(result) => result.message,
                                        Err(error) => error.to_string(),
                                    };
                                if !automatic_target {
                                    bail!("{browser_failure}");
                                }
                                tokio::task::spawn_blocking(move || {
                                    desktop::fill_focused(&value, submit)
                                })
                                .await
                                .context("Desktop input task failed")?
                                .with_context(|| {
                                    format!("Browser fill failed ({browser_failure}); desktop fallback also failed")
                                })?;
                                Ok("desktop".to_owned())
                            }
                            "desktop" => {
                                tokio::task::spawn_blocking(move || {
                                    desktop::fill_focused(&value, submit)
                                })
                                .await
                                .context("Desktop input task failed")??;
                                Ok("desktop".to_owned())
                            }
                            "terminal" => {
                                let session_id = terminal_session_id
                                    .context("No current terminal session; call terminal_start first")?;
                                self.terminal.fill_value(session_id, &value, submit)?;
                                self.bind_terminal_item(session_id, item_id);
                                Ok("terminal".to_owned())
                            }
                            _ => unreachable!(),
                        }
                    },
                )
                .await?;
            Ok(CredentialFillOutput {
                target: actual_target,
                module,
                submitted: submit,
            })
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "terminal_run",
        description = "Run one local shell command to completion and return stdout, stderr, and exit status in the same call. Use this for ordinary non-interactive local work; use PTY tools only when the process must stay interactive. stdin sends text directly without shell escaping or temporary files. command, stdin, and cwd may contain {{openkeylet:module}} placeholders, and secretEnv may inject a stored module when a child program needs an environment variable. For a redacted .env import template, pass envTemplate on its own so its project and module IDs can be resolved locally. OpenKeylet substitutes locally and redacts known values from output. Omit item when the session selected a compatible project or no hidden module is used. There is no sandbox, output limit, or default timeout.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<SshResponse>(),
        annotations(title = "Run a local command", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn terminal_run(
        &self,
        Parameters(input): Parameters<TerminalRunInput>,
    ) -> CallToolResult {
        let result: Result<SshResponse, String> = async {
            let mut input = input;
            let mut placeholder_names = HashSet::new();
            collect_secret_placeholders_text(&input.command, &mut placeholder_names)?;
            if let Some(stdin) = input.stdin.as_deref() {
                collect_secret_placeholders_text(stdin, &mut placeholder_names)?;
            }
            if let Some(cwd) = input.cwd.as_deref() {
                collect_secret_placeholders_text(cwd, &mut placeholder_names)?;
            }
            if let Some(template) = input.env_template.as_deref() {
                if !input.item.trim().is_empty()
                    || !placeholder_names.is_empty()
                    || !input.secret_env.is_empty()
                {
                    return Err(
                        "envTemplate cannot be combined with item, secretEnv, or inline placeholders"
                            .to_owned(),
                    );
                }
                let environment = crate::env_template::resolve(&self.vault, template)
                    .map_err(|error| error.to_string())?;
                let started = Instant::now();
                let result = crate::env_template::execute(
                    &environment,
                    &input.command,
                    input.cwd.as_deref(),
                    input.timeout_seconds,
                    input.stdin.as_deref(),
                )
                .await;
                let success = result.is_ok();
                let _ = self.vault.add_activity(NewActivity {
                    status: if success { "success" } else { "error" }.to_owned(),
                    source: self.activity_source(),
                    connection_name: ".env template".to_owned(),
                    action: "Run local command with .env template".to_owned(),
                    duration_ms: started.elapsed().as_millis() as u64,
                    error: if success { "" } else { "Local command failed" }.to_owned(),
                });
                return result.map_err(|error| error.to_string());
            }
            let uses_project = !input.item.trim().is_empty()
                || !placeholder_names.is_empty()
                || !input.secret_env.is_empty();
            if !uses_project {
                return execute_local(
                    None,
                    &input.command,
                    input.cwd.as_deref(),
                    input.timeout_seconds,
                    &HashMap::new(),
                    input.stdin.as_deref(),
                )
                .await
                .map_err(|error| error.to_string());
            }

            let mut connection = self.resolve_item_for_action(&input.item, Some("fill"))?;
            let item_id = connection.stored.id;
            if !placeholder_names.is_empty() {
                let bindings = placeholder_names
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<HashMap<_, _>>();
                let replacements =
                    self.resolve_secret_bindings(&mut connection, &bindings, &input.item)?;
                let command_replacements = replacements
                    .iter()
                    .map(|(name, value)| (name.clone(), default_shell_secret_literal(value)))
                    .collect::<HashMap<_, _>>();
                let mut used = HashSet::new();
                input.command =
                    interpolate_http_text(&input.command, &command_replacements, &mut used)?;
                if let Some(cwd) = input.cwd.as_mut() {
                    *cwd = interpolate_http_text(cwd, &replacements, &mut used)?;
                }
                if let Some(stdin) = input.stdin.as_mut() {
                    *stdin = interpolate_http_text(stdin, &replacements, &mut used)?;
                }
            }
            let secret_env =
                self.resolve_secret_bindings(&mut connection, &input.secret_env, &input.item)?;
            self.track(
                item_id,
                "Run local command".to_owned(),
                execute_local(
                    Some(&connection),
                    &input.command,
                    input.cwd.as_deref(),
                    input.timeout_seconds,
                    &secret_env,
                    input.stdin.as_deref(),
                ),
            )
            .await
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "terminal_start",
        description = "Start a program or script in a OpenKeylet-managed local PTY and make it the current terminal for this MCP session. Omit program to start the platform's default shell, or provide any program and argv; Windows .cmd and .bat scripts use the native command processor. Pass item once to bind the terminal so later terminal_write placeholders can omit it. command, args, and cwd may use a hidden module directly as {{openkeylet:password}} or {{openkeylet:module name}}; OpenKeylet substitutes it locally and redacts it from output. Use secretEnv only when the child program specifically needs an environment variable. command may contain the first line to submit immediately after startup. This is not a sandbox.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<TerminalOpenResult>(),
        annotations(title = "Open a managed terminal", read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true)
    )]
    async fn terminal_start(
        &self,
        Parameters(input): Parameters<TerminalStartInput>,
    ) -> CallToolResult {
        let mut input = input;
        let mut placeholder_names = HashSet::new();
        if let Err(error) = collect_secret_placeholders_text(&input.command, &mut placeholder_names)
        {
            return tool_error(error);
        }
        for argument in &input.args {
            if let Err(error) = collect_secret_placeholders_text(argument, &mut placeholder_names) {
                return tool_error(error);
            }
        }
        if let Some(cwd) = input.cwd.as_deref()
            && let Err(error) = collect_secret_placeholders_text(cwd, &mut placeholder_names)
        {
            return tool_error(error);
        }
        let uses_default_shell = input.program.trim().is_empty();
        let program = if uses_default_shell {
            default_terminal_program()
        } else {
            input.program
        };
        let mut environment = HashMap::new();
        let mut redacted_values = Vec::new();
        let mut injected_names = Vec::new();
        let tracked_item = if input.secret_env.is_empty()
            && placeholder_names.is_empty()
            && input.item.trim().is_empty()
        {
            None
        } else {
            let mut connection = match self.resolve_item_for_action(&input.item, Some("fill")) {
                Ok(connection) => connection,
                Err(error) => return tool_error(error),
            };
            if !placeholder_names.is_empty() {
                let bindings = placeholder_names
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<HashMap<_, _>>();
                let replacements =
                    match self.resolve_secret_bindings(&mut connection, &bindings, &input.item) {
                        Ok(replacements) => replacements,
                        Err(error) => return tool_error(error),
                    };
                let mut used = HashSet::new();
                let command_replacements = if uses_default_shell {
                    replacements
                        .iter()
                        .map(|(name, value)| (name.clone(), default_shell_secret_literal(value)))
                        .collect::<HashMap<_, _>>()
                } else {
                    replacements.clone()
                };
                input.command =
                    match interpolate_http_text(&input.command, &command_replacements, &mut used) {
                        Ok(command) => command,
                        Err(error) => return tool_error(error),
                    };
                for argument in &mut input.args {
                    *argument = match interpolate_http_text(argument, &replacements, &mut used) {
                        Ok(argument) => argument,
                        Err(error) => return tool_error(error),
                    };
                }
                if let Some(cwd) = input.cwd.as_mut() {
                    *cwd = match interpolate_http_text(cwd, &replacements, &mut used) {
                        Ok(cwd) => cwd,
                        Err(error) => return tool_error(error),
                    };
                }
                redacted_values.extend(replacements.into_values());
                injected_names.extend(placeholder_names);
            }
            for (name, module_hint) in input.secret_env {
                let module = match resolve_secret_module(&connection, &module_hint, &input.item) {
                    Ok(module) => module,
                    Err(error) => return tool_error(error),
                };
                let (_, kind, mut secret) =
                    match self.vault.agent_secret_value(connection.stored.id, &module) {
                        Ok(secret) => secret,
                        Err(error) => return tool_error(error.to_string()),
                    };
                if kind == "totp" {
                    secret = match current_totp(&secret) {
                        Ok(value) => value,
                        Err(error) => return tool_error(error.to_string()),
                    };
                }
                redacted_values.push(secret.clone());
                environment.insert(name.clone(), secret);
                injected_names.push(name);
            }
            injected_names.sort();
            injected_names.dedup();
            let action = if injected_names.is_empty() {
                "Start project terminal".to_owned()
            } else {
                format!(
                    "Start terminal using hidden module:{}",
                    injected_names.join("、")
                )
            };
            Some((connection.stored.id, action))
        };
        let terminal_item_id = tracked_item.as_ref().map(|(item_id, _)| *item_id);
        let start = async {
            self.terminal
                .open_with_env(
                    &program,
                    input.args,
                    input.cwd,
                    environment,
                    redacted_values,
                )
                .and_then(|opened| {
                    if !input.command.trim().is_empty()
                        && let Err(error) = self
                            .terminal
                            .input(opened.session_id, &submitted_terminal_line(&input.command))
                    {
                        let _ = self.terminal.close(opened.session_id);
                        return Err(error);
                    }
                    Ok(opened)
                })
        };
        let result = if let Some((item_id, action)) = tracked_item {
            self.track(item_id, action, start).await
        } else {
            start.await.map_err(|error| error.to_string())
        };
        if let (Ok(opened), Some(item_id)) = (&result, terminal_item_id) {
            self.bind_terminal_item(opened.session_id, item_id);
        }
        if let Ok(opened) = &result {
            self.bind_active_terminal(opened.session_id);
        }
        into_tool_result(result)
    }

    #[tool(
        name = "terminal_write",
        description = "Write interactive input to the current OpenKeylet-managed PTY. Omit sessionId for the current terminal; provide it only to switch among multiple terminals. text may contain {{openkeylet:password}} or {{openkeylet:module name}}; OpenKeylet inserts the raw value locally and redacts any echo, so later password, sudo, token, or TOTP prompts do not require a separate tool. item may be omitted after terminal_start or an earlier write bound the terminal. Set submit=true to press Enter without constructing a platform-specific newline. Use credential_fill for browser or desktop controls.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<OkOutput>(),
        annotations(title = "Write terminal input", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn terminal_write(
        &self,
        Parameters(input): Parameters<TerminalWriteInput>,
    ) -> CallToolResult {
        let session_id = match self.resolve_terminal_session_id(&input.session_id, true) {
            Ok(session_id) => session_id,
            Err(error) => return tool_error(error),
        };
        let text = if input.submit {
            submitted_terminal_line(&input.text)
        } else {
            input.text.clone()
        };
        let mut placeholder_names = HashSet::new();
        if let Err(error) = collect_secret_placeholders_text(&text, &mut placeholder_names) {
            return tool_error(error);
        }
        if placeholder_names.is_empty() {
            return into_tool_result(
                self.terminal
                    .input(session_id, &text)
                    .map(|_| OkOutput { ok: true })
                    .map_err(|error| error.to_string()),
            );
        }

        let result: Result<OkOutput, String> = async {
            let mut connection = self.resolve_terminal_item(session_id, &input.item)?;
            let item_id = connection.stored.id;
            let bindings = placeholder_names
                .iter()
                .map(|name| (name.clone(), name.clone()))
                .collect::<HashMap<_, _>>();
            let replacements =
                self.resolve_secret_bindings(&mut connection, &bindings, &input.item)?;
            let mut used = HashSet::new();
            let text = interpolate_http_text(&text, &replacements, &mut used)?;
            let redacted_values = replacements.into_values().collect::<Vec<_>>();
            let mut names = placeholder_names.into_iter().collect::<Vec<_>>();
            names.sort();
            self.track(
                item_id,
                format!("Write to terminal using hidden module:{}", names.join("、")),
                async {
                    self.terminal
                        .input_redacted(session_id, &text, &redacted_values)
                },
            )
            .await?;
            self.bind_terminal_item(session_id, item_id);
            Ok(OkOutput { ok: true })
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "terminal_read",
        description = "Read output from the current OpenKeylet-managed PTY. Omit sessionId for the current terminal; provide it only to switch among multiple terminals. Add until to accumulate output until that text appears or the process exits; this waits without a OpenKeylet deadline unless waitSeconds is set. Without until, waitSeconds waits until any output arrives. With neither, the read is immediate. OpenKeylet redacts values it filled and common encodings of those values.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<TerminalReadResult>(),
        annotations(title = "Read terminal output", read_only_hint = true, destructive_hint = false, idempotent_hint = false, open_world_hint = false)
    )]
    async fn terminal_read(
        &self,
        Parameters(input): Parameters<TerminalReadInput>,
    ) -> CallToolResult {
        let session_id = match self.resolve_terminal_session_id(&input.session_id, false) {
            Ok(session_id) => session_id,
            Err(error) => return tool_error(error),
        };
        let until = input.until.filter(|value| !value.is_empty());
        let deadline = input
            .wait_seconds
            .filter(|seconds| *seconds > 0)
            .map(|seconds| Instant::now() + Duration::from_secs(seconds));
        let mut output = String::new();
        let mut truncated = false;
        let result = loop {
            let mut read = match self.terminal.read(session_id) {
                Ok(read) => read,
                Err(error) => break Err(error.to_string()),
            };
            output.push_str(&read.output);
            truncated |= read.truncated;
            read.output = output.clone();
            read.truncated = truncated;
            if until.is_none() && deadline.is_none() {
                break Ok(read);
            }
            let reached = until
                .as_ref()
                .is_some_and(|expected| read.output.contains(expected));
            let has_enough_output = until.is_none() && !read.output.is_empty();
            let expired = deadline.is_some_and(|deadline| Instant::now() >= deadline);
            if reached || has_enough_output || !read.running || expired {
                break Ok(read);
            }
            let poll_interval = deadline.map_or(Duration::from_millis(50), |deadline| {
                Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now()))
            });
            tokio::time::sleep(poll_interval).await;
        };
        if matches!(&result, Ok(read) if !read.running) {
            self.unbind_terminal_item(session_id);
        }
        into_tool_result(result)
    }

    #[tool(
        name = "terminal_stop",
        description = "Close and clean up the current OpenKeylet-managed PTY, terminating the program if it is still running. Omit sessionId for the current terminal; provide it only to switch among multiple terminals.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<OkOutput>(),
        annotations(title = "Close a managed terminal", read_only_hint = false, destructive_hint = true, idempotent_hint = true, open_world_hint = false)
    )]
    async fn terminal_stop(
        &self,
        Parameters(input): Parameters<TerminalSessionInput>,
    ) -> CallToolResult {
        let result = self
            .resolve_terminal_session_id(&input.session_id, false)
            .and_then(|session_id| {
                self.terminal
                    .close(session_id)
                    .map_err(|error| error.to_string())?;
                self.unbind_terminal_item(session_id);
                self.clear_active_terminal(session_id);
                Ok(OkOutput { ok: true })
            });
        into_tool_result(result)
    }

    #[tool(
        name = "ssh_run",
        description = "Execute the requested command through OpenKeylet SSH. The project only needs a saved password, private key or matching SSH user certificate: host, port, and username may be supplied at runtime, with saved values as defaults and port 22 as the fallback. stdin sends text directly to the remote command without shell escaping or temporary files. Omit item after selecting a compatible project or when exactly one project stores SSH authentication. command, stdin, and cwd may use {{openkeylet:module}} placeholders; OpenKeylet substitutes and redacts them locally. Use secretEnv only when the remote program needs an environment variable. This grants full command authority and has no observation, diagnostic, restricted, or execution mode. For a saved jump route, call this tool with target project B directly. OpenKeylet automatically routes through jump project A and authenticates A and B with their own credentials. If a plain-key fingerprint or explicitly trusted host CA is missing or mismatched, stop and ask the user to verify it against a trusted source and confirm it through the OpenKeylet client or local ssh-trust CLI before retrying. Never auto-trust keys or bypass the route with a direct or nested ssh connection.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<SshResponse>(),
        annotations(title = "Execute an SSH command", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn ssh_run(&self, Parameters(input): Parameters<SshRunInput>) -> CallToolResult {
        let result: Result<SshResponse, String> = async {
            let mut input = input;
            let mut placeholder_names = HashSet::new();
            collect_secret_placeholders_text(&input.command, &mut placeholder_names)?;
            if let Some(stdin) = input.stdin.as_deref() {
                collect_secret_placeholders_text(stdin, &mut placeholder_names)?;
            }
            if let Some(cwd) = input.cwd.as_deref() {
                collect_secret_placeholders_text(cwd, &mut placeholder_names)?;
            }
            let mut connection = self.prepare_ssh_connection(
                &input.item,
                input.host.as_deref(),
                input.port,
                input.username.as_deref(),
            )?;
            let item_id = connection.stored.id;
            if !placeholder_names.is_empty() {
                let bindings = placeholder_names
                    .iter()
                    .map(|name| (name.clone(), name.clone()))
                    .collect::<HashMap<_, _>>();
                let replacements =
                    self.resolve_secret_bindings(&mut connection, &bindings, &input.item)?;
                let mut used = HashSet::new();
                let command_replacements = replacements
                    .iter()
                    .map(|(name, value)| (name.clone(), crate::executor::shell_quote(value)))
                    .collect::<HashMap<_, _>>();
                input.command =
                    interpolate_http_text(&input.command, &command_replacements, &mut used)?;
                if let Some(cwd) = input.cwd.as_mut() {
                    *cwd = interpolate_http_text(cwd, &replacements, &mut used)?;
                }
                if let Some(stdin) = input.stdin.as_mut() {
                    *stdin = interpolate_http_text(stdin, &replacements, &mut used)?;
                }
            }
            let secret_env =
                self.resolve_secret_bindings(&mut connection, &input.secret_env, &input.item)?;
            self.track(
                item_id,
                "Run SSH command".to_owned(),
                execute_ssh(
                    &self.vault,
                    &connection,
                    &input.command,
                    input.cwd.as_deref(),
                    input.timeout_seconds,
                    &secret_env,
                    input.stdin.as_deref(),
                ),
            )
            .await
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "ssh_upload",
        description = "Upload one local file or directory through OpenKeylet SFTP. The project only needs a saved password or private key; host, port, and username may be supplied at runtime, while saved values remain defaults and port 22 is the fallback. host selects an explicit address; alternateValues are never tried automatically. Directories are copied recursively. remotePath is optional and defaults to the local source name in the SSH login directory. Missing parents are created and an existing destination is replaced by default, even when its file/directory kind changed. For a saved jump route, call this tool with target project B directly. OpenKeylet automatically routes through jump project A and authenticates A and B with their own credentials. If a plain-key fingerprint or explicitly trusted host CA is missing or mismatched, stop and ask the user to verify it against a trusted source and confirm it through the OpenKeylet client or local ssh-trust CLI before retrying. Never auto-trust keys or bypass the route with a direct or nested ssh connection.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<SshTransferResponse>(),
        annotations(title = "Upload a path over SSH", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn ssh_upload(&self, Parameters(input): Parameters<SshUploadInput>) -> CallToolResult {
        let result: Result<SshTransferResponse, String> = async {
            let connection = self.prepare_ssh_connection(
                &input.item,
                input.host.as_deref(),
                input.port,
                input.username.as_deref(),
            )?;
            let item_id = connection.stored.id;
            let remote_path = if input.remote_path.trim().is_empty() {
                transfer_file_name(&input.local_path, "Local path")?
            } else {
                input.remote_path
            };
            self.track(
                item_id,
                "Upload SSH path".to_owned(),
                ssh_upload(
                    &self.vault,
                    &connection,
                    &input.local_path,
                    &remote_path,
                    input.overwrite,
                    input.timeout_seconds,
                ),
            )
            .await
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "ssh_download",
        description = "Download one remote file or directory through OpenKeylet SFTP. The project only needs a saved password or private key; host, port, and username may be supplied at runtime, while saved values remain defaults and port 22 is the fallback. host selects an explicit address; alternateValues are never tried automatically. Directories are copied recursively. localPath is optional and defaults to the remote source name in the MCP working directory. Missing parents are created and an existing destination is replaced by default, even when its file/directory kind changed. For a saved jump route, call this tool with target project B directly. OpenKeylet automatically routes through jump project A and authenticates A and B with their own credentials. If a plain-key fingerprint or explicitly trusted host CA is missing or mismatched, stop and ask the user to verify it against a trusted source and confirm it through the OpenKeylet client or local ssh-trust CLI before retrying. Never auto-trust keys or bypass the route with a direct or nested ssh connection.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<SshTransferResponse>(),
        annotations(title = "Download a path over SSH", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn ssh_download(
        &self,
        Parameters(input): Parameters<SshDownloadInput>,
    ) -> CallToolResult {
        let result: Result<SshTransferResponse, String> = async {
            let connection = self.prepare_ssh_connection(
                &input.item,
                input.host.as_deref(),
                input.port,
                input.username.as_deref(),
            )?;
            let item_id = connection.stored.id;
            let local_path = if input.local_path.trim().is_empty() {
                transfer_file_name(&input.remote_path, "Remote path")?
            } else {
                input.local_path
            };
            self.track(
                item_id,
                "Download SSH path".to_owned(),
                ssh_download(
                    &self.vault,
                    &connection,
                    &input.remote_path,
                    &local_path,
                    input.overwrite,
                    input.timeout_seconds,
                ),
            )
            .await
        }
        .await;
        into_tool_result(result)
    }

    #[tool(
        name = "http_send",
        description = "Send an authenticated HTTP request without returning hidden stored credentials. Use action_capabilities with httpRequest or nativeApi to prepare a concrete call. The typed workflow supports a fixed read operation and returns a continuation for the same project when there are more pages. Consult api_service_capabilities for service authentication and documentation. AWS aws-sigv4 signs standard supported commercial regional endpoints using bound Access Key ID, Secret Access Key and optional Session Token; the request region must match the saved region. Supply AWS Query Action/Version or JSON X-Amz-Target as documented, and process XML/JSON responses and pagination yourself. Native providers reject cross-provider targets, proxies and redirects; unsupported signing endpoints never silently fall back to Bearer. No credential refresh, IMDS, S3, streaming signatures or SigV4a. Existing selected CA projects remain unavailable for generic APIs. Omit item after selecting a compatible project, or pass its name. OCI projects provide general API signing, not a fixed list of operations: choose the method, path, query, headers and one body format from the OCI API documentation. Relative URLs use the default Compute base; absolute URLs select another OCI service in ociContext.region. Only HTTPS port 443 regional oraclecloud.com endpoints are allowed for OCI; redirects and proxies are disabled. Signing headers are generated locally. Use items_search to discover ociContext.region and tenancyOcid when needed. Resolve named resources through the appropriate OCI APIs, handle pagination using service response headers, and ask only when resource matches remain ambiguous. Do not treat an empty root-compartment result as all resources. OCI IAM controls permissions; do not bypass a denial. Each http_send sends one request. Typed workflows return pagination parameters without following pages automatically. Built-in API/Basic authentication is also injected automatically. Hidden ordinary modules may be referenced as {{openkeylet:password}} or {{openkeylet:service token}} in URL, headers, query or body values; secretBindings provides optional aliases. OCI signing fields follow ordinary module visibility and placeholder rules. Large responses may use saveResponseTo. Unrecognized generic HTTP services retain existing HTTP/HTTPS, proxy and redirect behavior. Recognized native providers enforce their own strict HTTPS target, no-proxy and no-redirect policies. No address failover occurs.",
        output_schema = rmcp::handler::server::tool::schema_for_type::<HttpToolResponse>(),
        annotations(title = "Send an authenticated API request", read_only_hint = false, destructive_hint = true, idempotent_hint = false, open_world_hint = true)
    )]
    async fn http_send(&self, Parameters(input): Parameters<HttpSendInput>) -> CallToolResult {
        let structured = input.workflow.is_some() || matches!(input.response_format, HttpResponseFormat::Structured);
        let result: Result<HttpToolResponse, String> = async {
            let mut input = input;
            let workflow = input.workflow.take();
            if workflow.is_some()
                && (input.item_id.is_none() || !input.item.is_empty() || !input.url.is_empty()
                    || input.method != "GET" || !input.query.is_empty() || !input.headers.is_empty()
                    || input.body.is_some() || !input.form.is_empty() || !input.files.is_empty()
                    || input.body_base64.is_some() || !input.secret_bindings.is_empty()
                    || input.save_response_to.is_some())
            {
                return Err("A native workflow requires itemId and supplies its own HTTP request parameters".to_owned());
            }
            include_http_secret_bindings(&mut input)?;
            let has_secret_bindings = !input.secret_bindings.is_empty();
            let (mut connection, reason) = self.resolve_action_project_with_reason(&input.item, input.item_id, Some("http"))?;
            self.vault.guard_agent_secret_use(connection.stored.id).map_err(|error| error.to_string())?;
            let item_id = connection.stored.id;
            let (request, source, mut lookup) = if let Some(workflow) = &workflow {
                let service = native_service(&connection, workflow).ok_or("Project does not support this native operation")?;
                if service.status != crate::api_provider::ConfigurationStatus::Ready {
                    return Err("Project is missing the native service configuration".to_owned());
                }
                let mut prepared = workflow.prepare(&connection).map_err(|issue| {
                    format!("{} {}", if issue.invalid { "Invalid" } else { "Missing" }, issue.field)
                })?;
                prepared.request.timeout_seconds = input.timeout_seconds.or(Some(30));
                prepared.request.max_response_bytes = input.max_response_bytes;
                protect_native_context(&mut connection);
                (prepared.request, TargetSource::Saved, prepared.lookup)
            } else {
                let source = http_target_source(&input.url);
                self.prepare_http_secret_bindings(&mut connection, &mut input)?;
                (ApiRequestInput {
                    url: input.url,
                    method: input.method,
                    query: input.query,
                    headers: input.headers,
                    body: input.body,
                    form: input.form,
                    files: input.files,
                    body_base64: input.body_base64,
                    timeout_seconds: input.timeout_seconds,
                    max_response_bytes: input.max_response_bytes,
                    save_response_to: input.save_response_to,
                    overwrite_response_file: input.overwrite_response_file,
                }, source, None)
            };
            let mut action = describe_api_request(&connection.stored, &request);
            if has_secret_bindings || workflow.is_some() {
                action = crate::policy::redact(action, &connection.stored, &connection.secrets);
            }
            let mut response = self.track(item_id, action, execute_api(&connection, request)).await?;
            let pagination = workflow.as_ref().and_then(|workflow| workflow.next_page(&response)).and_then(|next| {
                let next_call = native_next_call(item_id, &next);
                let secrets = zeroize::Zeroizing::new(connection.secrets.non_empty_values());
                let candidates = zeroize::Zeroizing::new(crate::policy::redaction_candidates(&secrets));
                (redact_http_json(next_call.arguments.clone(), &candidates) == next_call.arguments)
                    .then_some(HttpPagination { next_call })
            });
            let (project, target, data, error) = if structured {
                let target = describe_http_target(&connection, source, url::Url::parse(&response.url).ok());
                response.url = target.url.clone().unwrap_or_else(|| "[REDACTED]".to_owned());
                let data = structured_http_data(&response, &connection);
                if (200..300).contains(&response.status)
                    && let (Some(lookup), Some(data)) = (&mut lookup, &data)
                {
                    lookup.include_matches(data);
                }
                let error = (!(200..300).contains(&response.status)).then(|| HttpRemoteError {
                    code: "remoteHttpError",
                    status: response.status,
                    request_id: ["x-request-id", "x-github-request-id", "opc-request-id",
                        "x-amzn-requestid", "x-amz-request-id", "cf-ray"]
                        .iter()
                        .find_map(|name| response.headers.get(*name).cloned()),
                });
                (Some(SelectedProject::new(&connection.stored, reason)), Some(target), data, error)
            } else {
                (None, None, None, None)
            };
            Ok(HttpToolResponse {
                status: response.status,
                status_text: response.status_text,
                url: response.url,
                headers: response.headers,
                body: data.is_none().then_some(response.body),
                truncated: response.truncated,
                saved_to: response.saved_to,
                bytes_transferred: response.bytes_transferred,
                project,
                target,
                data,
                error,
                pagination,
                lookup,
            })
        }
        .await;
        match result {
            Ok(value) if structured => {
                let summary = format!("HTTP {}", value.status);
                match serde_json::to_value(value) {
                    Ok(value) => {
                        let mut result = CallToolResult::structured(value);
                        result.content = vec![ContentBlock::text(summary)];
                        result
                    }
                    Err(_) => tool_error("OpenKeylet could not serialize the tool result."),
                }
            }
            result => into_tool_result(result),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for VaultMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("openkeylet", env!("CARGO_PKG_VERSION")).with_title("OpenKeylet"))
            .with_instructions(
            "OpenKeylet lets agents use locally saved credentials without receiving hidden plaintext. For a ready request with the exact project and target, use action_capabilities with httpRequest or nativeApi. The nativeApi workflows cover GitHub commits, OCI compartments and running instances, AWS EC2 running instances, Cloudflare zones and DNS, DigitalOcean Droplets, and Hetzner servers. API provider workflow: api_service_capabilities identifies a concrete API service, protocol support, required bindings and official documentation. AWS native aws-sigv4 uses awsBindings stable module IDs for serviceUrl, region, accessKeyId, secretAccessKey and optional sessionToken. Create ordinary modules, then bind returned IDs under normal write grants. The agent chooses operations, method, action/version, parameters and pagination; OpenKeylet validates supported provider/region targets, signs final bytes and sends one request without redirects/proxies. Native AWS supports a documented standard endpoint subset; S3, SigV4a, streaming, presigning and automatic credential refresh are excluded. Other vendor signatures and OAuth acquisition are not provided merely because a URL is recognized. GitHub/Cloudflare/DigitalOcean/Hetzner guides describe their distinct API/token boundaries. Do not infer all services from a company brand or send private keys/credential JSON as Bearer tokens. Certificate workflow: certificate_capabilities reports separately owner-granted CA kinds and TTL limits; certificate_inspect and certificate_verify operate on public material, and certificate_check_key compares a saved key locally. certificate_issue accepts public subject keys/CSRs only, a UUID requestId, concrete identities, purpose and ttlSeconds. Reuse the UUID only for identical retries; current grants are checked again before replay. CA-marked projects cannot be delegated through fill, HTTP, SSH, placeholders, secretEnv or terminals even if a module was marked visible. Agent cannot register/unmark a CA or edit signing grants. Existing CA bindings remain protected after module/type edits. Store SSH leaves as a standard sshCertificate module beside its matching privateKey, choose sshAuthType certificate; jump endpoints authenticate independently. Host CA trust requires explicit user confirmation of signer CA fingerprint for each exact item/host/port using the client or ssh-trust set-ca; never auto-trust from a probe. TLS verification does not check revocation. This does not sandbox arbitrary programs already having local-user file access. An explicit item or unique items_search result becomes the current project; later compatible actions may omit item, and a terminal keeps its own binding. When the user names a project and the action is clear, call it directly. Use items_search only for discovery, selection, inspection, or genuine ambiguity. For OCI, http_send is a general signed request executor. The agent reads the OCI API documentation and chooses operations, methods, parameters, pagination and filtering; OpenKeylet validates the target, signs locally and returns the response. items_search exposes non-secret ociContext.region and tenancyOcid. Use the relevant OCI APIs to discover named compartments or resources rather than asking the user for OCIDs that can be retrieved; ask when matches are ambiguous. Use an absolute regional OCI service URL when the API is not under the default Compute base. OCI requests stay within the configured region on HTTPS port 443, never follow redirects, and never use proxies. Resource permissions remain with OCI IAM. Do not bypass IAM denials or claim an empty root-compartment page means the tenancy has no instances. Each call sends one request; handle service pagination and filter returned data as required by the user. OCI signing fields are ordinary modules bound by stable module IDs. Module visibility controls all plaintext returned to the agent, including the signing key and OCI request context. Hidden modules can still be used locally without revealing plaintext. credential_fill understands common module phrases and may infer a sole secret; omit target to use the current running OpenKeylet terminal, otherwise a connected browser or foreground control, and use submit=true to complete the login. terminal_start makes the opened PTY current, so terminal_write, terminal_read, terminal_stop, and terminal credential fills should omit sessionId unless deliberately switching among multiple terminals. Use terminal_run for one-shot local commands and PTY tools only for persistent interaction. Direct {{openkeylet:module}} placeholders work in commands, stdin, paths, terminal input, SSH, and HTTP values; OpenKeylet substitutes and redacts them locally. terminal_run.stdin and ssh_run.stdin pass scripts, JSON, or other text without shell escaping or temporary files. Host and service URL modules may save other addresses in alternateValues; value is the default. Omitted SSH host or HTTP URL uses that default, relative HTTP paths use the default service URL, and explicit host or absolute URL selects another target only within the selected native provider target policy where applicable. Addresses share project credentials and SSH port; there is no automatic failover. A password or private-key project can use SSH without saved target metadata: pass host, optional port, and username at runtime; saved values are defaults and port 22 is the fallback. For a saved SSH jump route, call ssh_run, ssh_upload, or ssh_download with target project B directly. OpenKeylet automatically routes through jump project A and authenticates each endpoint with its own credentials. items_search exposes the saved route; items_create/items_update configure sshJumpItemId under the existing write permissions and revision rules. If a plain-key fingerprint or explicitly trusted host CA is missing or mismatched, stop and ask the user to verify it against a trusted source and confirm it through the OpenKeylet client or local ssh-trust CLI, confirming A before probing B when needed. Never auto-trust keys or bypass the route with a direct or nested ssh connection. Use secretEnv only when a program needs an environment variable, and HTTP secretBindings only for aliases. Never ask the user to reveal a hidden value. terminal_read.until waits without polling and has no deadline unless waitSeconds is supplied. For vault writing, consult vault_write_capabilities, then items_create/items_update. Writing requires desktop folder authorization, uses stable project/module IDs and expectedRevision, and cannot change visibility. Never echo submitted secrets. OpenKeylet has no observation, diagnostic, restricted, or execution modes.",
        )
    }

    fn on_initialized(
        &self,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = ()> + MaybeSendFuture + '_ {
        if let Some(info) = context.peer.peer_info() {
            let client_name = display_client_name(&info.client_info.name);
            if let Ok(mut stored_name) = self.client_name.write() {
                *stored_name = client_name;
            }
        }
        std::future::ready(())
    }
}

fn display_client_name(raw_name: &str) -> String {
    let normalized = raw_name.trim().to_ascii_lowercase();
    if normalized.contains("claude") {
        "CLAUDE CODE".to_owned()
    } else if normalized.contains("codex") {
        "CODEX".to_owned()
    } else if normalized.contains("cursor") {
        "CURSOR".to_owned()
    } else if normalized.contains("opencode") {
        "OPENCODE".to_owned()
    } else if normalized.contains("chatgpt") {
        "CHATGPT".to_owned()
    } else {
        raw_name
            .trim()
            .chars()
            .filter(|character| !character.is_control())
            .take(24)
            .collect::<String>()
            .to_uppercase()
    }
}

pub async fn serve_stdio(vault: Vault) -> Result<()> {
    let data_dir = vault.data_dir().to_path_buf();
    crate::runtime_epoch::monitor_until_invalidated(data_dir)?;
    let server = VaultMcp::new(vault);
    let browser = server.browser.clone();
    browser.sync().await;
    let result = async {
        let service = server.serve(stdio()).await?;
        service.waiting().await?;
        Ok(())
    }
    .await;
    browser.stop().await;
    result
}

pub fn render_config(format: &str) -> Result<String> {
    let executable = launcher_executable()?;
    let executable = path_text(&executable);
    match format {
        "stdio-json" => Ok(serde_json::to_string_pretty(&json!({
            "mcpServers": {"openkeylet": {"command": executable, "args": ["mcp", "stdio"]}}
        }))?),
        "stdio-toml" => Ok(format!(
            "[mcp_servers.openkeylet]\ncommand = \"{}\"\nargs = [\"mcp\", \"stdio\"]",
            toml_escape(&executable)
        )),
        _ => bail!("Unknown configuration format:{format}"),
    }
}

pub fn launcher_executable() -> Result<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    if let Some(path) = validated_linux_launcher(
        std::env::var_os("OPENKEYLET_LAUNCHER_PATH")
            .or_else(|| std::env::var_os("KRU_LAUNCHER_PATH"))
            .map(std::path::PathBuf::from),
        std::env::var_os("APPIMAGE").map(std::path::PathBuf::from),
    ) {
        return Ok(path);
    }
    #[cfg(target_os = "linux")]
    if let Some(path) = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from)
        && path.is_absolute()
        && path.is_file()
    {
        return Ok(path);
    }
    let current =
        std::env::current_exe().context("Unable to determine the OpenKeylet executable path")?;
    Ok(stable_launcher_executable(current))
}

fn stable_launcher_executable(current: std::path::PathBuf) -> std::path::PathBuf {
    let Some(parent) = current.parent() else {
        return current;
    };
    if !parent
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("deps"))
    {
        return current;
    }
    let Some(file_name) = current.file_name() else {
        return current;
    };
    let Some(release_dir) = parent.parent() else {
        return current;
    };
    let stable = release_dir.join(file_name);
    if stable.is_file() { stable } else { current }
}

#[cfg(target_os = "linux")]
fn validated_linux_launcher(
    candidate: Option<std::path::PathBuf>,
    appimage: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    let candidate = candidate?;
    let appimage = appimage?;
    let metadata = candidate.metadata().ok()?;
    if !candidate.is_absolute()
        || !appimage.is_absolute()
        || !metadata.is_file()
        || metadata.permissions().mode() & 0o111 == 0
        || !appimage.is_file()
        || candidate.parent()?.canonicalize().ok()? != appimage.parent()?.canonicalize().ok()?
    {
        return None;
    }
    Some(candidate)
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionInput, ItemModule, NamedSecrets, SecretBundle};
    use tempfile::tempdir;

    #[tokio::test]
    async fn oci_mcp_exposes_request_context_and_protects_signing_key() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        vault
            .save_oci_connection(
                serde_json::from_value(serde_json::json!({
                    "name": "OCI test",
                    "modules": [
                        {"kind":"customSecret", "name":"auth_token"},
                        {"kind":"username"},
                        {"kind":"password"},
                        {"kind":"customSecret", "name":"project_note", "agentVisible":true},
                        {"kind":"url", "value":"https://wrong.example"}
                    ],
                    "secrets": {"password":"fixture password","namedSecrets":{"username":"fixture account","auth_token":"hidden fixture","project_note":"visible fixture"}}
                })).unwrap(),
                None,
                crate::oci::tests::config(),
                Some(crate::oci::tests::KEY),
                None,
            )
            .unwrap();
        let server = VaultMcp::new(vault);
        let discovered = server
            .search_items(ItemsSearchInput {
                query: Some("OCI test".into()),
            })
            .unwrap();
        assert_eq!(discovered.items[0].actions, ["credential_fill", "http_send", "certificate_check_key"]);
        let context = discovered.items[0].oci_context.as_ref().unwrap();
        assert_eq!(context.region.as_deref(), Some("ap-tokyo-1"));
        assert_eq!(context.tenancy_ocid, Some(crate::oci::tests::config().tenancy_ocid));
        assert_eq!(serde_json::to_value(context).unwrap(), serde_json::json!({
            "region": "ap-tokyo-1",
            "tenancyOcid": "ocid1.tenancy.oc1..test"
        }));
        let discovered_json = serde_json::to_string(&discovered).unwrap();
        for hidden in ["PRIVATE KEY", "hidden fixture", "fixture password", "fixture account", "ocid1.user.oc1..test", "00:00:00"] {
            assert!(!discovered_json.contains(hidden));
        }
        let modules = &discovered.items[0].modules;
        assert_eq!(modules.len(), 10);
        assert!(modules.iter().any(|m| m.kind == "username" && m.configured));
        assert!(
            modules
                .iter()
                .any(|m| m.kind == "password" && m.configured && m.value.is_none())
        );
        assert!(modules.iter().any(|module| module.name == "auth_token"
            && module.configured
            && module.value.is_none()));
        assert!(modules.iter().any(|module| module.name == "project_note"
            && module.value.as_deref() == Some("visible fixture")));
        assert!(modules.iter().any(|module| module.kind == "url"
            && module.value.as_deref()
                == Some("https://iaas.ap-tokyo-1.oraclecloud.com/20160918/")));
        assert!(
            server
                .resolve_item_for_action("OCI test", Some("fill"))
                .is_ok()
        );

        let id = discovered.items[0].id.parse().unwrap();
        let mut input: ConnectionInput = serde_json::from_value(serde_json::to_value(server.vault.get_connection(id).unwrap().stored.portable()).unwrap()).unwrap();
        let bindings = input.oci_bindings.clone().unwrap();
        for module in &mut input.modules {
            if [bindings.region, bindings.tenancy_ocid].contains(&Some(module.id)) { module.agent_visible = Some(false); }
            if Some(module.id) == bindings.private_key { module.agent_visible = Some(true); }
        }
        server.vault.save_connection(input).unwrap();
        let visible_key = server.search_items(ItemsSearchInput { query: Some("OCI test".into()) }).unwrap();
        assert!(visible_key.items[0].oci_context.is_none());
        assert!(visible_key.items[0].modules.iter().any(|module| Some(module.id.as_str()) == bindings.private_key.map(|id| id.to_string()).as_deref() && module.value.as_deref() == Some(crate::oci::tests::KEY)));
        assert!(server.resolve_item_for_action("OCI test", Some("ssh")).is_err());

        let mut connection = server.resolve_item_for_action("OCI test", Some("http")).unwrap();
        let mut request: HttpSendInput = serde_json::from_value(serde_json::json!({
            "item": "OCI test", "method": "POST", "url": "newApiOperation",
            "body": {"token":"{{openkeylet:auth_token}}"},
            "secretBindings": {"auth_token":"auth_token"}
        })).unwrap();
        server.prepare_http_secret_bindings(&mut connection, &mut request).unwrap();
        assert_eq!(request.body.unwrap()["token"], "hidden fixture");
        assert_eq!(crate::policy::redact("hidden fixture".into(), &connection.stored, &connection.secrets), "[REDACTED]");

        for request in [
            serde_json::json!({"item": "OCI test", "method": "POST", "url": "https://outside.example/newApiOperation"}),
            serde_json::json!({"item": "OCI test", "url": "instances?compartmentId={{openkeylet:oci}}"}),
        ] {
            let result = server
                .http_send(Parameters(serde_json::from_value(request).unwrap()))
                .await;
            assert_eq!(result.is_error, Some(true));
            let error = serde_json::to_string(&result).unwrap();
            assert!(!error.contains("PRIVATE KEY"));
            assert!(!error.contains("hidden fixture"));
        }
    }

    #[test]
    fn old_and_new_secret_templates_can_be_mixed_without_recursive_substitution() {
        let input = "{{kru:password}}|{{openkeylet:token}}|{{kru:token}}";
        let replacements = HashMap::from([
            (
                "password".to_owned(),
                "secret-{{openkeylet:token}}".to_owned(),
            ),
            ("token".to_owned(), "token-value".to_owned()),
        ]);
        let mut modules = HashSet::new();
        collect_secret_placeholders_text(input, &mut modules).unwrap();
        assert_eq!(
            modules,
            HashSet::from(["password".to_owned(), "token".to_owned()])
        );
        let mut used = HashSet::new();
        assert_eq!(
            interpolate_http_text(input, &replacements, &mut used).unwrap(),
            "secret-{{openkeylet:token}}|token-value|token-value"
        );
        assert_eq!(used, modules);
        assert!(collect_secret_placeholders_text("{{kru:password}", &mut modules).is_err());
        assert!(interpolate_http_text("{{kru:missing}}", &replacements, &mut used).is_err());
    }

    fn structured_value(result: &CallToolResult) -> serde_json::Value {
        assert_eq!(result.is_error, Some(false));
        let structured = result.structured_content.clone().unwrap();
        let wire = serde_json::to_value(result).unwrap();
        let text = wire["content"][0]["text"].as_str().unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text).unwrap(),
            structured
        );
        structured
    }

    fn env_template_project(vault: &Vault, name: &str, value: &str) -> (Uuid, Uuid) {
        let project = vault
            .save_connection(
                serde_json::from_value(json!({
                    "name": name,
                    "itemType": "custom",
                    "modules": [{ "kind": "customSecret", "name": "Value", "agentVisible": false }],
                    "secrets": { "namedSecrets": { "Value": value } },
                }))
                .unwrap(),
            )
            .unwrap();
        (project.id, project.modules[0].id)
    }

    #[tokio::test]
    async fn action_capabilities_previews_shared_selection_without_changing_session() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let (first, _) = env_template_project(&vault, "First project", "first-secret");
        let mcp = VaultMcp::new(vault.clone());
        let tool = mcp.tool_router.get("action_capabilities").unwrap();
        assert_eq!(tool.input_schema["type"], "object");
        assert_eq!(tool.output_schema.as_ref().unwrap()["type"], "object");
        let mcp_ref = &mcp;
        let preview = |input| async move {
            let input = serde_json::from_value(input).unwrap();
            structured_value(&mcp_ref.action_capabilities(Parameters(input)).await)
        };

        let unique = preview(json!({ "action": "selectProject", "capability": "fill" })).await;
        assert_eq!(unique["project"], json!({ "id": first, "name": "First project", "selection": "unique" }));
        assert_eq!(unique["status"], "selected");

        let (second, _) = env_template_project(&vault, "Second project", "second-secret");
        let choose = preview(json!({ "action": "selectProject", "capability": "fill" })).await;
        assert_eq!(choose["status"], "chooseProject");
        assert_eq!(choose["candidates"].as_array().unwrap().len(), 2);
        assert_eq!(choose["missingInputs"][0]["name"], "item");
        assert!(!choose.to_string().contains("-secret"));

        mcp.resolve_action_project("", Some(second), Some("fill")).unwrap();
        let explicit = preview(json!({ "action": "selectProject", "capability": "fill", "itemId": first })).await;
        assert_eq!(explicit["project"]["id"], json!(first));
        assert_eq!(explicit["project"]["selection"], "explicit");
        let session = preview(json!({ "action": "selectProject", "capability": "fill" })).await;
        assert_eq!(session["project"]["id"], json!(second));
        assert_eq!(session["project"]["selection"], "session");

        vault.set_connection_enabled(first, false).unwrap();
        let stale = preview(json!({ "action": "selectProject", "capability": "fill", "itemId": first })).await;
        assert_eq!(stale["status"], "unavailable");
        assert_eq!(stale["code"], "projectNotFound");
        assert_eq!(stale["candidates"], json!([{ "id": second, "name": "Second project" }]));
        assert!(mcp.resolve_action_project("", Some(first), Some("fill")).is_err());
        vault.set_connection_enabled(second, false).unwrap();
        let no_http = preview(json!({ "action": "selectProject", "capability": "http" })).await;
        assert_eq!(no_http["code"], "noCompatibleProject");
    }

    #[tokio::test]
    async fn action_capabilities_holds_hidden_urls_and_rejects_unresolved_bindings() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let saved = vault
            .save_connection(
                serde_json::from_value(json!({
                    "name": "Hidden URL",
                    "itemType": "custom",
                    "modules": [
                        { "kind": "url", "value": "https://fixture.invalid/private", "agentVisible": false },
                        { "kind": "customSecret", "name": "Value", "agentVisible": false },
                    ],
                    "secrets": { "namedSecrets": { "Value": "fixture-secret" } },
                }))
                .unwrap(),
            )
            .unwrap();
        let mcp = VaultMcp::new(vault);
        let preview = |body| {
            let input = serde_json::from_value(json!({
                "action": "httpRequest",
                "itemId": saved.id,
                "url": "relative",
                "method": "POST",
                "body": body,
            }))
            .unwrap();
            mcp.action_capabilities(Parameters(input))
        };
        let response = structured_value(&preview("{{openkeylet:Value}}").await);
        assert_eq!(response["status"], "ready");
        assert_eq!(response["target"], json!({ "source": "relative" }));
        assert_eq!(response["nextCall"]["arguments"]["url"], "relative");
        assert_eq!(response["nextCall"]["arguments"]["body"], "{{openkeylet:Value}}");
        let wire = response.to_string();
        assert!(!wire.contains("fixture.invalid"));
        assert!(!wire.contains("fixture-secret"));

        let response = structured_value(&preview("{{openkeylet:does-not-exist}}").await);
        assert_eq!(response["status"], "needsInput");
        assert_eq!(response["code"], "missingSecretBinding");
        assert!(response.get("nextCall").is_none());
    }

    #[test]
    fn structured_http_data_redacts_escaped_secrets_in_json_keys_and_values() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let secret = r#"fixture"token\tail"#;
        let (project, _) = env_template_project(&vault, "JSON token", secret);
        let connection = vault.get_connection(project).unwrap();
        let response = ApiResponse {
            status: 200,
            status_text: "OK".into(),
            url: "https://fixture.invalid/".into(),
            headers: HashMap::from([("content-type".into(), "application/json".into())]),
            body: json!({ secret: [secret, {"keep": true}] }).to_string(),
            truncated: false,
            saved_to: None,
            bytes_transferred: None,
        };
        assert!(!response.body.contains(secret));
        assert_eq!(
            structured_http_data(&response, &connection).unwrap(),
            json!({ "[REDACTED]": ["[REDACTED]", {"keep": true}] })
        );
    }

    #[tokio::test]
    async fn terminal_run_env_template_injects_exact_project_bindings_and_redacts_output() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let (first_id, first_module) =
            env_template_project(&vault, "Named differently", "synthetic-alpha\nsynthetic-beta");
        let (second_id, second_module) =
            env_template_project(&vault, "Other project", "synthetic-gamma");
        env_template_project(&vault, "FIXTURE_ALPHA", "synthetic-decoy");
        let template = format!(
            "FIXTURE_ALPHA={}\nFIXTURE_BETA={}\n",
            crate::env_template::placeholder(first_id, first_module),
            crate::env_template::placeholder(second_id, second_module),
        );
        #[cfg(windows)]
        let command = "if ($env:FIXTURE_ALPHA -ne ('synthetic-' + 'alpha' + [char]10 + 'synthetic-beta') -or $env:FIXTURE_BETA -ne ('synthetic-' + 'gamma')) { exit 11 }; [Console]::Write($env:FIXTURE_ALPHA); [Console]::Error.Write($env:FIXTURE_BETA)";
        #[cfg(not(windows))]
        let command = "test \"$FIXTURE_ALPHA\" = \"$(printf 'synthetic-%s\\nsynthetic-beta' alpha)\" && test \"$FIXTURE_BETA\" = \"$(printf 'synthetic-%s' gamma)\" || exit 11; printf '%s' \"$FIXTURE_ALPHA\"; printf '%s' \"$FIXTURE_BETA\" >&2";
        let mcp = VaultMcp::new(vault.clone());
        let input = serde_json::from_value(json!({
            "command": command, "envTemplate": template, "timeoutSeconds": 10,
        }))
        .unwrap();
        let result = structured_value(&mcp.terminal_run(Parameters(input)).await);
        assert_eq!(result["exitCode"], 0);
        assert_eq!(result["stdout"], "[REDACTED]");
        assert_eq!(result["stderr"], "[REDACTED]");
        let activities = vault.activities().unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].action, "Run local command with .env template");
        assert!(!serde_json::to_string(&activities).unwrap().contains("synthetic"));
    }

    #[tokio::test]
    async fn terminal_run_env_template_rejects_invalid_or_duplicate_bindings_before_spawning() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let (id, module) = env_template_project(&vault, "Valid project", "synthetic-allowed");
        let (disabled_id, disabled_module) =
            env_template_project(&vault, "Disabled project", "synthetic-disabled");
        vault.set_connection_enabled(disabled_id, false).unwrap();
        let reference = crate::env_template::placeholder(id, module);
        let stale_module = crate::env_template::placeholder(id, Uuid::new_v4());
        let disabled = crate::env_template::placeholder(disabled_id, disabled_module);
        let mcp = VaultMcp::new(vault.clone());
        for template in [
            format!("FIXTURE_ALPHA={reference}\nFIXTURE_BETA={stale_module}"),
            format!("FIXTURE_ALPHA={reference}\nfixture_alpha={reference}"),
            format!("FIXTURE_ALPHA={reference}\nFIXTURE_BETA={disabled}"),
            format!("FIXTURE_ALPHA={reference}\nFIXTURE_BETA=synthetic-raw"),
        ] {
            let input = serde_json::from_value(json!({
                "command": "echo ran > marker.txt",
                "cwd": directory.path().to_string_lossy(),
                "envTemplate": template,
                "timeoutSeconds": 10,
            }))
            .unwrap();
            let result = mcp.terminal_run(Parameters(input)).await;
            assert_eq!(result.is_error, Some(true));
            assert!(!directory.path().join("marker.txt").exists());
            assert!(!serde_json::to_string(&result).unwrap().contains("synthetic-"));
        }
        assert!(vault.activities().unwrap().is_empty());
    }

    #[test]
    fn development_binary_registers_the_stable_release_launcher() {
        let directory = tempdir().unwrap();
        let release = directory.path().join("release");
        let deps = release.join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let stable = release.join(if cfg!(windows) {
            "openkeylet.exe"
        } else {
            "openkeylet"
        });
        let development = deps.join(if cfg!(windows) {
            "openkeylet.exe"
        } else {
            "openkeylet"
        });
        std::fs::write(&stable, "stable").unwrap();
        std::fs::write(&development, "development").unwrap();

        assert_eq!(stable_launcher_executable(development), stable);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn portable_launcher_must_be_executable_and_beside_appimage() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir().unwrap();
        let launcher = directory.path().join("openkeylet");
        let appimage = directory.path().join("OpenKeylet.AppImage");
        std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
        std::fs::write(&appimage, "appimage").unwrap();
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            validated_linux_launcher(Some(launcher.clone()), Some(appimage.clone())),
            Some(launcher.clone())
        );

        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            validated_linux_launcher(Some(launcher), Some(appimage)),
            None
        );
    }

    #[tokio::test]
    async fn item_list_only_exposes_values_enabled_by_user() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut named = NamedSecrets::default();
        named.insert("username".into(), "mcp-visible-user-marker".into());
        named.insert("password".into(), "mcp-hidden-password-marker".into());
        named.insert("totp".into(), "JBSWY3DPEHPK3PXP".into());
        let mut secrets = SecretBundle::default();
        secrets.named_secrets = named;
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
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "username".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(true),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "password".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "totp".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(true),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "host".into(),
                        name: String::new(),
                        value: "ssh.example.test".into(),
                        agent_visible: Some(true),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "port".into(),
                        name: String::new(),
                        value: "22".into(),
                        agent_visible: Some(true),
                    },
                ],
                name: "test login".into(),
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
            })
            .unwrap();

        let mcp = VaultMcp::new(vault.clone());
        let output = mcp
            .items_search(Parameters(ItemsSearchInput::default()))
            .await;
        let listed = structured_value(&output);
        let text = serde_json::to_string(&listed).unwrap();
        assert!(listed["items"][0].get("ociContext").is_none());
        assert!(text.contains("username"));
        assert!(text.contains("password"));
        assert_eq!(
            listed["items"][0]["actions"],
            json!([
                "credential_fill",
                "ssh_run",
                "ssh_upload",
                "ssh_download",
                "http_send"
            ])
        );
        for removed in [
            "type",
            "capabilities",
            "description",
            "fields",
            "target",
        ] {
            assert!(
                listed["items"][0].get(removed).is_none(),
                "legacy field {removed} remains"
            );
        }
        assert!(
            listed["items"][0]["modules"][0].get("secret").is_none(),
            "module output still exposes the removed secret projection"
        );
        assert!(text.contains("mcp-visible-user-marker"));
        assert!(!text.contains("mcp-hidden-password-marker"));
        assert!(!text.contains("JBSWY3DPEHPK3PXP"));
        assert_eq!(
            listed["items"][0]["modules"]
                .as_array()
                .unwrap()
                .iter()
                .find(|module| module["kind"] == "totp")
                .unwrap()["value"]
                .as_str()
                .unwrap()
                .len(),
            6
        );

        let exact = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("TEST LOGIN".into()),
            }))
            .await,
        );
        assert_eq!(exact["items"].as_array().unwrap().len(), 1);
        assert_eq!(exact["items"][0]["name"], "test login");

        let partial = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("login".into()),
            }))
            .await,
        );
        assert_eq!(partial["items"].as_array().unwrap().len(), 1);

        let natural_phrase = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("Use TEST_LOGIN in OpenKeylet MCP to finish signing in".into()),
            }))
            .await,
        );
        assert_eq!(natural_phrase["items"].as_array().unwrap().len(), 1);
        assert_eq!(natural_phrase["items"][0]["name"], "test login");
        assert_eq!(
            mcp.resolve_item_for_action(
                "Use TEST_LOGIN in OpenKeylet MCP to finish signing in",
                None,
            )
            .unwrap()
            .stored
            .name,
            "test login"
        );

        let mut api_named = NamedSecrets::default();
        api_named.insert("apiCredential".into(), "mcp-api-marker".into());
        let mut api_secrets = SecretBundle::default();
        api_secrets.named_secrets = api_named;
        let api_saved = vault
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
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "apiCredential".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "url".into(),
                        name: String::new(),
                        value: "https://example.test".into(),
                        agent_visible: Some(true),
                    },
                ],
                name: "test login api".into(),
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
                secrets: api_secrets,
            })
            .unwrap();
        assert_eq!(
            mcp.resolve_item_for_action("use test login api in OpenKeylet MCP", None)
                .unwrap()
                .stored
                .name,
            "test login api"
        );
        assert_eq!(
            mcp.resolve_item_for_action("use test login api in OpenKeylet MCP", Some("ssh"))
                .unwrap()
                .stored
                .name,
            "test login"
        );
        assert_eq!(
            mcp.resolve_item_for_action("", Some("http"))
                .unwrap()
                .stored
                .name,
            "test login"
        );
        assert_eq!(
            mcp.resolve_item_for_action("", None).unwrap().stored.name,
            "test login"
        );

        let selected = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("use test login in OpenKeylet MCP".into()),
            }))
            .await,
        );
        assert_eq!(selected["items"].as_array().unwrap().len(), 1);
        assert_eq!(
            mcp.resolve_item_for_action("", None).unwrap().stored.name,
            "test login"
        );

        let missing = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("does not exist".into()),
            }))
            .await,
        );
        assert!(missing["items"].as_array().unwrap().is_empty());
        assert!(mcp.resolve_item_for_action("", None).is_err());

        vault.set_connection_enabled(saved.id, false).unwrap();
        vault.set_connection_enabled(api_saved.id, false).unwrap();
        let hidden = VaultMcp::new(vault)
            .items_search(Parameters(ItemsSearchInput::default()))
            .await;
        let hidden = serde_json::to_string(&structured_value(&hidden)).unwrap();
        assert!(!hidden.contains("test login"));
        assert!(!hidden.contains("username"));
        assert!(!hidden.contains("password"));
    }

    #[test]
    fn password_item_can_use_runtime_ssh_target_without_saved_target_modules() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut secrets = SecretBundle::default();
        secrets.password = Some("runtime-ssh-password-marker".into());
        let public = vault
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
                modules: vec![ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    agent_visible: Some(false),
                    ..Default::default()
                }],
                name: "Reusable SSH Secret".into(),
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
            })
            .unwrap();
        assert_eq!(public.capabilities, vec!["fill", "ssh", "http"]);
        assert!(!public.can_test);
        let mcp = VaultMcp::new(vault);

        let listed = mcp
            .search_items(ItemsSearchInput {
                query: Some("Reusable SSH Secret".into()),
            })
            .unwrap();
        assert_eq!(
            listed.items[0].actions,
            vec![
                "credential_fill",
                "ssh_run",
                "ssh_upload",
                "ssh_download",
                "http_send"
            ]
        );

        let connection = mcp
            .prepare_ssh_connection(
                "Reusable SSH Secret",
                Some("runtime.example.test"),
                None,
                Some("runtime-user"),
            )
            .unwrap();
        assert_eq!(connection.stored.host, "runtime.example.test");
        assert_eq!(connection.stored.port, 22);
        assert_eq!(connection.secrets.get("username"), Some("runtime-user"));
        assert_eq!(
            connection.secrets.get("password"),
            Some("runtime-ssh-password-marker")
        );
    }

    #[test]
    fn http_secret_placeholders_cover_request_values_without_rescanning_secrets() {
        let mut input = serde_json::from_value::<HttpSendInput>(json!({
            "url": "https://example.test/{{openkeylet:token}}",
            "headers": {"X-Token": "Bearer {{openkeylet:token}}"},
            "query": {"token": "{{openkeylet:token}}", "nested": ["{{openkeylet:second}}"]},
            "body": {"credential": "{{openkeylet:token}}"},
            "form": {"credential": "prefix-{{openkeylet:second}}"}
        }))
        .unwrap();
        let replacements = HashMap::from([
            ("token".to_owned(), "value-{{openkeylet:second}}".to_owned()),
            ("second".to_owned(), "other-value".to_owned()),
        ]);

        assert_eq!(
            collect_http_secret_placeholders(&input).unwrap(),
            HashSet::from(["token".to_owned(), "second".to_owned()])
        );

        let used = interpolate_http_input(&mut input, &replacements).unwrap();

        assert_eq!(
            input.url,
            "https://example.test/value-{{openkeylet:second}}"
        );
        assert_eq!(
            input.headers["X-Token"],
            "Bearer value-{{openkeylet:second}}"
        );
        assert_eq!(input.query["token"], "value-{{openkeylet:second}}");
        assert_eq!(input.query["nested"][0], "other-value");
        assert_eq!(
            input.body.unwrap()["credential"],
            "value-{{openkeylet:second}}"
        );
        assert_eq!(input.form["credential"], "prefix-other-value");
        assert_eq!(
            used,
            HashSet::from(["token".to_owned(), "second".to_owned()])
        );

        let mut used = HashSet::new();
        assert!(
            interpolate_http_text("{{openkeylet:missing}}", &HashMap::new(), &mut used).is_err()
        );
        assert!(interpolate_http_text("{{openkeylet:token}", &replacements, &mut used).is_err());
    }

    #[tokio::test]
    async fn http_send_uses_any_hidden_module_and_redacts_dynamic_totp() {
        type Seen = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

        async fn echo(
            axum::extract::State(seen): axum::extract::State<Seen>,
            axum::extract::OriginalUri(uri): axum::extract::OriginalUri,
            headers: axum::http::HeaderMap,
            body: axum::body::Bytes,
        ) -> String {
            let otp = headers
                .get("x-openkeylet-otp")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned();
            let received = vec![
                otp,
                uri.to_string(),
                String::from_utf8_lossy(&body).into_owned(),
            ];
            *seen.lock().unwrap() = received.clone();
            received.join("|")
        }

        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let secret = "dynamic-http-secret-marker-7319";
        let totp_seed = "JBSWY3DPEHPK3PXP";
        let mut named = NamedSecrets::default();
        named.insert("service password".into(), secret.into());
        named.insert("totp".into(), totp_seed.into());
        let mut secrets = SecretBundle::default();
        secrets.named_secrets = named;
        vault
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
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "customSecret".into(),
                        name: "service password".into(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "totp".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                ],
                name: "dynamic HTTP item".into(),
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
            })
            .unwrap();

        let seen = Seen::default();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let application = axum::Router::new()
            .route("/echo/{secret}", axum::routing::post(echo))
            .with_state(seen.clone());
        let server = tokio::spawn(async move { axum::serve(listener, application).await });

        let mcp = VaultMcp::new(vault.clone());
        let listed = structured_value(
            &mcp.items_search(Parameters(ItemsSearchInput {
                query: Some("dynamic HTTP item".into()),
            }))
            .await,
        );
        assert_eq!(
            listed["items"][0]["actions"],
            json!(["credential_fill", "http_send"])
        );

        let request = serde_json::from_value::<HttpSendInput>(json!({
            "item": "dynamic HTTP item",
            "url": format!("http://{address}/echo/{{{{openkeylet:auth}}}}"),
            "method": "POST",
            "headers": {"X-OpenKeylet-Otp": "{{openkeylet:otp}}"},
            "query": {"credential": "{{openkeylet:auth}}"},
            "body": {"credential": "{{openkeylet:auth}}"},
            "secretBindings": {
                "auth": "service password",
                "otp": "totp",
                "unused": "service password"
            }
        }))
        .unwrap();
        let result = structured_value(&mcp.http_send(Parameters(request)).await);
        let output = serde_json::to_string(&result).unwrap();
        let received = seen.lock().unwrap().clone();
        let otp = &received[0];

        assert_eq!(otp.len(), 6);
        assert!(otp.chars().all(|character| character.is_ascii_digit()));
        assert_ne!(otp, totp_seed);
        assert!(received[1].contains(secret));
        assert!(received[2].contains(secret));
        assert!(output.contains("[REDACTED]"));
        assert!(!output.contains(secret));
        assert!(!output.contains(totp_seed));
        assert!(!output.contains(otp));

        let direct_request = serde_json::from_value::<HttpSendInput>(json!({
            "item": "dynamic HTTP item",
            "url": format!("http://{address}/echo/{{{{openkeylet:service password}}}}"),
            "method": "POST",
            "headers": {"X-OpenKeylet-Otp": "{{openkeylet:totp}}"},
            "query": {"credential": "{{openkeylet:service password}}"},
            "body": {"credential": "{{openkeylet:service password}}"}
        }))
        .unwrap();
        let direct_result = structured_value(&mcp.http_send(Parameters(direct_request)).await);
        let direct_output = serde_json::to_string(&direct_result).unwrap();
        let direct_received = seen.lock().unwrap().clone();
        let direct_otp = &direct_received[0];

        assert_eq!(direct_otp.len(), 6);
        assert!(
            direct_otp
                .chars()
                .all(|character| character.is_ascii_digit())
        );
        assert_ne!(direct_otp, totp_seed);
        assert!(direct_received[1].contains(secret));
        assert!(direct_received[2].contains(secret));
        assert!(direct_output.contains("[REDACTED]"));
        assert!(!direct_output.contains(secret));
        assert!(!direct_output.contains(totp_seed));
        assert!(!direct_output.contains(direct_otp));

        let plain_request = serde_json::from_value::<HttpSendInput>(json!({
            "item": "dynamic HTTP item",
            "url": format!("http://{address}/echo/plain"),
            "method": "POST",
            "body": {"ping": true}
        }))
        .unwrap();
        let plain_result = structured_value(&mcp.http_send(Parameters(plain_request)).await);
        assert_eq!(plain_result["status"], 200);
        assert!(plain_result["body"].as_str().unwrap().contains("plain"));

        let activities = serde_json::to_string(&vault.activities().unwrap()).unwrap();
        assert!(activities.contains("[REDACTED]"));
        assert!(!activities.contains(secret));
        assert!(!activities.contains(totp_seed));
        assert!(!activities.contains(otp));
        server.abort();
    }

    #[tokio::test]
    async fn credential_fill_infers_single_secret_and_terminal_target() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut secrets = SecretBundle::default();
        secrets.password = Some("single-secret-terminal-marker".into());
        vault
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
                modules: vec![ItemModule {
                    alternate_values: Vec::new(),
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    name: String::new(),
                    value: String::new(),
                    agent_visible: Some(false),
                }],
                name: "single secret".into(),
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
            })
            .unwrap();

        let mcp = VaultMcp::new(vault);
        #[cfg(windows)]
        let (program, args) = ("cmd.exe", vec!["/Q".to_owned()]);
        #[cfg(not(windows))]
        let (program, args) = ("/bin/sh", Vec::new());
        let terminal = mcp.terminal.open(program, args, None).unwrap();
        mcp.bind_active_terminal(terminal.session_id);
        let output = mcp
            .credential_fill(Parameters(CredentialFillInput {
                item: String::new(),
                item_id: None,
                module: String::new(),
                target: String::new(),
                session_id: None,
                submit: false,
            }))
            .await;
        let filled = structured_value(&output);
        assert_eq!(filled["module"], "password");
        assert_eq!(filled["target"], "terminal");
        assert_eq!(filled["submitted"], false);
        assert!(
            !serde_json::to_string(&filled)
                .unwrap()
                .contains("single-secret-terminal-marker")
        );
        mcp.terminal.close(terminal.session_id).unwrap();
    }

    #[tokio::test]
    async fn credential_fill_resolves_module_and_target_from_common_phrases() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut named = NamedSecrets::default();
        named.insert("username".into(), "natural-user-marker".into());
        named.insert("password".into(), "natural-password-marker".into());
        let mut secrets = SecretBundle::default();
        secrets.named_secrets = named;
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
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "username".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                    ItemModule {
                        alternate_values: Vec::new(),
                        id: uuid::Uuid::nil(),
                        kind: "password".into(),
                        name: String::new(),
                        value: String::new(),
                        agent_visible: Some(false),
                    },
                ],
                name: "multi login".into(),
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
            })
            .unwrap();

        let connection = vault.get_connection(saved.id).unwrap();
        assert_eq!(
            resolve_secret_module(&connection, "user name", "").unwrap(),
            "username"
        );
        assert!(resolve_secret_module(&connection, "", "multi login").is_err());

        let mcp = VaultMcp::new(vault);
        #[cfg(windows)]
        let (program, args) = ("cmd.exe", vec!["/Q".to_owned()]);
        #[cfg(not(windows))]
        let (program, args) = ("/bin/sh", Vec::new());
        let terminal = mcp.terminal.open(program, args, None).unwrap();
        let output = mcp
            .credential_fill(Parameters(CredentialFillInput {
                item: "use the password from multi login in OpenKeylet MCP".into(),
                item_id: None,
                module: String::new(),
                target: "PTY".into(),
                session_id: Some(terminal.session_id.to_string()),
                submit: false,
            }))
            .await;
        let filled = structured_value(&output);
        assert_eq!(filled["module"], "password");
        assert_eq!(filled["target"], "terminal");
        assert!(
            !serde_json::to_string(&filled)
                .unwrap()
                .contains("natural-password-marker")
        );
        mcp.terminal.close(terminal.session_id).unwrap();
    }

    #[tokio::test]
    async fn terminal_start_without_program_opens_the_platform_shell() {
        let directory = tempdir().unwrap();
        let mcp = VaultMcp::new(Vault::open(directory.path().join("vault")).unwrap());
        let output = mcp
            .terminal_start(Parameters(TerminalStartInput {
                item: String::new(),
                program: String::new(),
                args: Vec::new(),
                cwd: None,
                command: String::new(),
                secret_env: HashMap::new(),
            }))
            .await;
        let opened = structured_value(&output);
        let session_id = Uuid::parse_str(opened["sessionId"].as_str().unwrap()).unwrap();
        assert!(!opened["executable"].as_str().unwrap().is_empty());
        #[cfg(windows)]
        assert!(
            opened["executable"]
                .as_str()
                .unwrap()
                .to_ascii_lowercase()
                .ends_with("powershell.exe")
        );
        let stopped = mcp
            .terminal_stop(Parameters(TerminalSessionInput {
                session_id: String::new(),
            }))
            .await;
        assert_ne!(stopped.is_error, Some(true));
        assert!(!mcp.terminal.contains(session_id).unwrap());
    }

    #[tokio::test]
    async fn terminal_start_can_submit_first_command_and_read_can_wait() {
        let directory = tempdir().unwrap();
        let mcp = VaultMcp::new(Vault::open(directory.path().join("vault")).unwrap());
        #[cfg(windows)]
        let (program, args) = ("cmd.exe".to_owned(), vec!["/D".to_owned(), "/Q".to_owned()]);
        #[cfg(not(windows))]
        let (program, args) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "sleep 0.3; IFS= read -r line; printf '%s' \"$line\"".to_owned(),
            ],
        );
        let opened = structured_value(
            &mcp.terminal_start(Parameters(TerminalStartInput {
                item: String::new(),
                program,
                args,
                cwd: None,
                command: if cfg!(windows) {
                    "echo OPENKEYLET_INITIAL_COMMAND_MARKER".to_owned()
                } else {
                    "OPENKEYLET_INITIAL_COMMAND_MARKER".to_owned()
                },
                secret_env: HashMap::new(),
            }))
            .await,
        );
        let session_id = opened["sessionId"].as_str().unwrap().to_owned();
        let read = structured_value(
            &mcp.terminal_read(Parameters(TerminalReadInput {
                session_id: String::new(),
                wait_seconds: Some(10),
                until: Some("OPENKEYLET_INITIAL_COMMAND_MARKER".into()),
            }))
            .await,
        );
        assert!(
            read["output"]
                .as_str()
                .unwrap()
                .contains("OPENKEYLET_INITIAL_COMMAND_MARKER")
        );
        mcp.terminal
            .close(Uuid::parse_str(&session_id).unwrap())
            .unwrap();
    }

    #[tokio::test]
    async fn terminal_start_injects_and_redacts_hidden_environment_modules() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let secret = "terminal secret ' with spaces $ marker-7391";
        let mut secrets = SecretBundle::default();
        secrets.password = Some(secret.to_owned());
        vault
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
                modules: vec![ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    agent_visible: Some(false),
                    ..Default::default()
                }],
                name: "CLI Login".into(),
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
            })
            .unwrap();
        let mut other_secrets = SecretBundle::default();
        other_secrets.password = Some("other-terminal-secret-marker".into());
        vault
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
                modules: vec![ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "password".into(),
                    agent_visible: Some(false),
                    ..Default::default()
                }],
                name: "Other CLI Login".into(),
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
                secrets: other_secrets,
            })
            .unwrap();
        let mcp = VaultMcp::new(vault);
        #[cfg(windows)]
        let one_shot_command = "$openkeyletInput=[Console]::In.ReadToEnd(); Write-Output {{openkeylet:password}}; Write-Output $openkeyletInput; [Console]::Error.WriteLine($env:OPENKEYLET_HIDDEN_ENV)";
        #[cfg(not(windows))]
        let one_shot_command = "printf '%s\n' {{openkeylet:password}}; cat; printf '%s\n' \"$OPENKEYLET_HIDDEN_ENV\" >&2";
        let one_shot = structured_value(
            &mcp.terminal_run(Parameters(TerminalRunInput {
                item: "use CLI Login in OpenKeylet MCP".into(),
                command: one_shot_command.into(),
                stdin: Some("stdin={{openkeylet:password}}".into()),
                cwd: None,
                timeout_seconds: Some(10),
                secret_env: HashMap::from([("OPENKEYLET_HIDDEN_ENV".into(), "password".into())]),
                env_template: None,
            }))
            .await,
        );
        assert_eq!(one_shot["exitCode"], 0);
        assert!(one_shot["stdout"].as_str().unwrap().contains("[REDACTED]"));
        assert!(one_shot["stderr"].as_str().unwrap().contains("[REDACTED]"));
        assert!(!one_shot.to_string().contains(secret));

        #[cfg(windows)]
        let (program, args) = (
            "powershell.exe".to_owned(),
            vec![
                "-NoProfile".to_owned(),
                "-Command".to_owned(),
                "[Console]::Write($env:OPENKEYLET_HIDDEN_ENV)".to_owned(),
            ],
        );
        #[cfg(not(windows))]
        let (program, args) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "printf '%s' \"$OPENKEYLET_HIDDEN_ENV\"".to_owned(),
            ],
        );
        let opened = structured_value(
            &mcp.terminal_start(Parameters(TerminalStartInput {
                item: "use CLI Login in OpenKeylet MCP".into(),
                program,
                args,
                cwd: None,
                command: String::new(),
                secret_env: HashMap::from([("OPENKEYLET_HIDDEN_ENV".into(), "password".into())]),
            }))
            .await,
        );
        assert!(!opened.to_string().contains(secret));
        let session_id = opened["sessionId"].as_str().unwrap().to_owned();
        let read = structured_value(
            &mcp.terminal_read(Parameters(TerminalReadInput {
                session_id: session_id.clone(),
                wait_seconds: Some(2),
                until: Some("[REDACTED]".into()),
            }))
            .await,
        );
        let terminal_output = read["output"].as_str().unwrap();
        assert!(terminal_output.contains("[REDACTED]"));
        assert!(!terminal_output.contains(secret));
        mcp.terminal
            .close(Uuid::parse_str(&session_id).unwrap())
            .unwrap();

        #[cfg(windows)]
        let direct_command = "Write-Output {{openkeylet:password}}; exit";
        #[cfg(not(windows))]
        let direct_command = "printf '%s\\n' {{openkeylet:password}}; exit";
        let direct = structured_value(
            &mcp.terminal_start(Parameters(TerminalStartInput {
                item: "use CLI Login in OpenKeylet MCP".into(),
                program: String::new(),
                args: Vec::new(),
                cwd: None,
                command: direct_command.into(),
                secret_env: HashMap::new(),
            }))
            .await,
        );
        assert!(!direct.to_string().contains(secret));
        let direct_session_id = direct["sessionId"].as_str().unwrap().to_owned();
        let direct_read = structured_value(
            &mcp.terminal_read(Parameters(TerminalReadInput {
                session_id: direct_session_id.clone(),
                wait_seconds: Some(10),
                until: Some("[REDACTED]".into()),
            }))
            .await,
        );
        let direct_output = direct_read["output"].as_str().unwrap();
        assert!(!direct_output.contains(secret));
        assert!(direct_output.contains("[REDACTED]"));
        mcp.terminal
            .close(Uuid::parse_str(&direct_session_id).unwrap())
            .unwrap();

        #[cfg(windows)]
        let (interactive_program, interactive_args, interactive_text) = (
            "powershell.exe".to_owned(),
            vec![
                "-NoProfile".to_owned(),
                "-Command".to_owned(),
                "Start-Sleep -Milliseconds 300; [Console]::ReadLine() | Write-Output".to_owned(),
            ],
            "{{openkeylet:password}}".to_owned(),
        );
        #[cfg(not(windows))]
        let (interactive_program, interactive_args, interactive_text) = (
            "/bin/sh".to_owned(),
            vec![
                "-c".to_owned(),
                "sleep 0.3; IFS= read -r line; printf '%s' \"$line\"".to_owned(),
            ],
            "{{openkeylet:password}}".to_owned(),
        );
        let interactive = structured_value(
            &mcp.terminal_start(Parameters(TerminalStartInput {
                item: "use CLI Login in OpenKeylet MCP".into(),
                program: interactive_program,
                args: interactive_args,
                cwd: None,
                command: String::new(),
                secret_env: HashMap::new(),
            }))
            .await,
        );
        let interactive_session_id = interactive["sessionId"].as_str().unwrap().to_owned();
        let written = mcp
            .terminal_write(Parameters(TerminalWriteInput {
                item: String::new(),
                session_id: String::new(),
                text: interactive_text,
                submit: true,
            }))
            .await;
        assert_ne!(written.is_error, Some(true));
        assert!(!serde_json::to_string(&written).unwrap().contains(secret));
        let interactive_read = structured_value(
            &mcp.terminal_read(Parameters(TerminalReadInput {
                session_id: String::new(),
                wait_seconds: Some(5),
                until: Some("[REDACTED]".into()),
            }))
            .await,
        );
        let interactive_output = interactive_read["output"].as_str().unwrap();
        assert!(!interactive_output.contains(secret));
        assert!(interactive_output.contains("[REDACTED]"));
        let stopped = mcp
            .terminal_stop(Parameters(TerminalSessionInput {
                session_id: String::new(),
            }))
            .await;
        assert_ne!(stopped.is_error, Some(true));
        let interactive_id = Uuid::parse_str(&interactive_session_id).unwrap();
        assert!(
            !mcp.terminal_items
                .read()
                .unwrap()
                .contains_key(&interactive_id)
        );
    }

    #[tokio::test]
    async fn write_tools_round_trip_and_hide_malformed_values() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path().join("vault")).unwrap();
        vault.update_settings_patch(serde_json::from_value(serde_json::json!({"agentWriteEnabled":true,"agentWriteFolders":["Agent"]})).unwrap()).unwrap();
        let mcp = VaultMcp::new(vault.clone());
        let input = serde_json::from_value(serde_json::json!({"name":"MCP synthetic","itemType":"login","folder":"Agent","modules":[{"kind":"password","value":"NEVER-ECHO"}]})).unwrap();
        let result = mcp.items_create(Parameters(WriteRequest(input))).await;
        assert_ne!(result.is_error,Some(true));
        assert!(!serde_json::to_string(&result).unwrap().contains("NEVER-ECHO"));
        let found=mcp.search_items(ItemsSearchInput {query:Some("MCP synthetic".into())}).unwrap();
        assert_eq!(found.items.len(),1);
        assert!(!found.items[0].id.is_empty());
        let input=serde_json::from_value(serde_json::json!({"id":found.items[0].id,"expectedRevision":found.items[0].revision,"modules":[{"id":found.items[0].modules[0].id,"value":"REPLACEMENT"}]})).unwrap();
        let result=mcp.items_update(Parameters(WriteRequest(input))).await;
        assert_ne!(result.is_error,Some(true));
        assert!(!serde_json::to_string(&result).unwrap().contains("REPLACEMENT"));
        let malformed=serde_json::from_value::<WriteRequest<crate::vault_write::CreateInput>>(serde_json::json!({"name":"x","folder":"Agent","itemType":"ssh","modules":"DO-NOT-ECHO"}));
        let error=malformed.err().unwrap().to_string();
        assert!(!error.contains("DO-NOT-ECHO"));
    }

    #[test]
    fn tool_contracts_have_output_schemas_and_annotations() {
        let router = VaultMcp::tool_router();
        let expected = [
            ("items_search", true, false, true, false),
            ("vault_write_capabilities", true, false, true, false),
            ("items_create", false, false, false, false),
            ("items_update", false, true, false, false),
            ("credential_fill", false, false, false, true),
            ("terminal_run", false, true, false, true),
            ("terminal_start", false, false, false, true),
            ("terminal_write", false, true, false, true),
            ("terminal_read", true, false, false, false),
            ("terminal_stop", false, true, true, false),
            ("ssh_run", false, true, false, true),
            ("ssh_upload", false, true, false, true),
            ("ssh_download", false, true, false, true),
            ("http_send", false, true, false, true),
        ];
        for (name, read_only, destructive, idempotent, open_world) in expected {
            let tool = router.get(name).unwrap();
            assert!(
                tool.output_schema.is_some(),
                "{name} needs an output schema"
            );
            let annotations = tool.annotations.as_ref().unwrap();
            assert_eq!(annotations.read_only_hint, Some(read_only), "{name}");
            assert_eq!(annotations.destructive_hint, Some(destructive), "{name}");
            assert_eq!(annotations.idempotent_hint, Some(idempotent), "{name}");
            assert_eq!(annotations.open_world_hint, Some(open_world), "{name}");
        }
        for removed in [
            "vault_items_list",
            "secret_fill",
            "terminal_open",
            "terminal_input",
            "terminal_close",
            "ssh_execute",
            "api_request",
        ] {
            assert!(
                router.get(removed).is_none(),
                "removed tool {removed} is still registered"
            );
        }
        let schema = serde_json::to_value(router.get("items_search").unwrap().output_schema.as_ref().unwrap()).unwrap();
        assert!(schema.to_string().contains("ociContext"));
        assert!(schema.to_string().contains("tenancyOcid"));
        assert!(schema.to_string().contains("ociBindings"));
    }

    #[test]
    fn renamed_arguments_are_rejected() {
        assert!(
            serde_json::from_value::<CredentialFillInput>(json!({
                "item": "test login",
                "field": "password",
                "target": "browser"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<SshRunInput>(json!({
                "connectionId": Uuid::new_v4().to_string(),
                "command": "hostname"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<HttpSendInput>(json!({
                "itemId": Uuid::new_v4().to_string(),
                "url": "https://api.example.test/v1/health",
                "path": "health"
            }))
            .is_err()
        );
    }

    #[test]
    fn low_friction_defaults_submit_only_when_requested_and_replace_transfer_targets() {
        let shell = serde_json::from_value::<TerminalStartInput>(json!({})).unwrap();
        assert!(shell.program.is_empty());
        assert!(shell.args.is_empty());
        assert!(shell.cwd.is_none());
        assert!(shell.command.is_empty());
        assert!(shell.item.is_empty());
        assert!(shell.secret_env.is_empty());

        let inferred_fill = serde_json::from_value::<CredentialFillInput>(json!({})).unwrap();
        assert!(inferred_fill.item.is_empty());
        assert!(inferred_fill.module.is_empty());
        assert!(inferred_fill.target.is_empty());
        assert!(inferred_fill.session_id.is_none());
        assert!(!inferred_fill.submit);

        let current_write = serde_json::from_value::<TerminalWriteInput>(json!({
            "text": "continue",
            "submit": true
        }))
        .unwrap();
        assert!(current_write.session_id.is_empty());
        let current_read = serde_json::from_value::<TerminalReadInput>(json!({
            "until": "ready"
        }))
        .unwrap();
        assert!(current_read.session_id.is_empty());
        let current_stop = serde_json::from_value::<TerminalSessionInput>(json!({})).unwrap();
        assert!(current_stop.session_id.is_empty());

        let fill = serde_json::from_value::<CredentialFillInput>(json!({
            "item": "test login",
            "module": "password",
            "target": "browser"
        }))
        .unwrap();
        assert!(!fill.submit);

        let ssh = serde_json::from_value::<SshRunInput>(json!({
            "command": "hostname"
        }))
        .unwrap();
        assert!(ssh.item.is_empty());
        assert!(ssh.host.is_none());
        assert!(ssh.port.is_none());
        assert!(ssh.username.is_none());

        let upload = serde_json::from_value::<SshUploadInput>(json!({
            "localPath": "C:\\fixture.bin"
        }))
        .unwrap();
        assert!(upload.item.is_empty());
        assert!(upload.host.is_none());
        assert!(upload.port.is_none());
        assert!(upload.username.is_none());
        assert!(upload.remote_path.is_empty());
        assert!(upload.overwrite);
        assert_eq!(
            transfer_file_name(&upload.local_path, "Local path").unwrap(),
            "fixture.bin"
        );

        let download = serde_json::from_value::<SshDownloadInput>(json!({
            "remotePath": "/tmp/fixture.bin"
        }))
        .unwrap();
        assert!(download.item.is_empty());
        assert!(download.host.is_none());
        assert!(download.port.is_none());
        assert!(download.username.is_none());
        assert!(download.local_path.is_empty());
        assert!(download.overwrite);
        assert_eq!(
            transfer_file_name(&download.remote_path, "Remote path").unwrap(),
            "fixture.bin"
        );

        let response = serde_json::from_value::<HttpSendInput>(json!({
            "url": "https://example.test/file",
            "saveResponseTo": "C:\\fixture.bin"
        }))
        .unwrap();
        assert!(response.item.is_empty());
        assert!(response.secret_bindings.is_empty());
        assert!(response.overwrite_response_file);
    }

    #[tokio::test]
    async fn business_failures_are_tool_errors() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let result = VaultMcp::new(vault)
            .terminal_read(Parameters(TerminalReadInput {
                session_id: "not-a-session".into(),
                wait_seconds: None,
                until: None,
            }))
            .await;
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
    }
    #[test]
    fn ssh_address_defaults_and_explicit_targets_preserve_the_project_port() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().into()).unwrap();
        let input = serde_json::from_value(serde_json::json!({"name":"multi-host","itemType":"ssh","modules":[
            {"kind":"host","value":"a.example","alternateValues":["b.example"]},
            {"kind":"port","value":"2222"},{"kind":"password"},{"kind":"username"}],
            "secrets":{"password":"mock-only","namedSecrets":{"username":"mock-user"}}
        })).unwrap();
        vault.save_connection(input).unwrap();
        let mcp = VaultMcp::new(vault);
        let default = mcp.prepare_ssh_connection("multi-host", None, None, None).unwrap();
        assert_eq!(default.stored.host, "a.example");
        assert_eq!(default.stored.port, 2222);
        let explicit = mcp.prepare_ssh_connection("multi-host", Some("b.example"), None, None).unwrap();
        assert_eq!(explicit.stored.host, "b.example");
        assert_eq!(explicit.stored.port, 2222);
        assert_eq!(explicit.secrets.get("password"), Some("mock-only"));
    }

}

#[cfg(test)]
#[path = "mcp_certificate_tests.rs"]
mod certificate_tests;
