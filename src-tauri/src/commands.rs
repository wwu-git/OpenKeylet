#[path = "env_import.rs"]
mod env_import;
#[path = "ssh_config_import.rs"]
pub(crate) mod ssh_config_import;
#[path = "owner_session.rs"]
mod owner_session;

use owner_session::OwnerSession;
use crate::{
    agent_registry::{AgentActionResult, AgentClientStatus, AgentRegistry},
    backup,
    browser::BrowserBridge,
    clipboard::ClipboardGuard,
    executor,
    github_backup::{self, BackupStatus, ConnectInput, GithubBackup, RestoreResult},
    gui_instance::GuiInstance,
    mcp,
    model::{
        AppState, ConnectionInput, ImportSummary, NewActivity, OwnerEditorDraft, OwnerLockState,
        OwnerSecretView, PortableConnection, PublicConnection, SecretBundle, SettingsPatch,
    },
    storage::app_data_dir,
    vault::Vault,
};
use anyhow::{Context, Result};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, State, WebviewWindow,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct AppRuntime {
    _gui_instance: GuiInstance,
    vault: Vault,
    executable: String,
    browser: BrowserBridge,
    owner: Mutex<OwnerState>,
    app: AppHandle,
    clipboard: ClipboardGuard,
    github_backup: Mutex<GithubBackup>,
    agent_registry: AgentRegistry,
    tray_available: AtomicBool,
}

struct PendingImport {
    id: String,
    created: Instant,
    encrypted: crate::model::SecretEnvelope,
}

enum SelectableImportState {
    Preparing(Uuid),
    Ready(PendingImport),
}

impl SelectableImportState {
    fn matches(&self, id: Uuid) -> bool {
        match self {
            Self::Preparing(current) => *current == id,
            Self::Ready(pending) => pending.id == id.to_string(),
        }
    }
}

#[derive(Default)]
struct OwnerState {
    session: OwnerSession,
    pending_private_key: Option<PendingPrivateKey>,
    pending_oci_key: Option<(Uuid, zeroize::Zeroizing<String>)>,
    pending_import: Option<PendingImport>,
    pending_backup: Option<SelectableImportState>,
    pending_env: Option<SelectableImportState>,
    env_undo: Option<env_import::UndoBatch>,
    pending_ssh: Option<SelectableImportState>,
    ssh_undo: Option<env_import::UndoBatch>,
}

impl OwnerState {
    fn lock(&mut self, runtime: &AppRuntime) {
        self.session.lock();
        self.pending_private_key = None;
        self.pending_oci_key = None;
        self.pending_import = None;
        self.pending_backup = None;
        self.pending_env = None;
        self.env_undo = None;
        self.pending_ssh = None;
        self.ssh_undo = None;
        runtime.clipboard.clear_on_lock(&runtime.app);
    }
}

impl AppRuntime {
    async fn owner_state(&self) -> Result<(tokio::sync::MutexGuard<'_, OwnerState>, bool), String> {
        let mut owner = self.owner.lock().await;
        let configured = self.vault.owner_pin_configured().map_err(command_error)?;
        if configured && owner.session.expired(Instant::now()) {
            owner.lock(self);
            let _ = self.app.emit("owner-locked", ());
        }
        Ok((owner, configured))
    }

    async fn authorized_owner(&self) -> Result<tokio::sync::MutexGuard<'_, OwnerState>, String> {
        let (owner, configured) = self.owner_state().await?;
        if owner.session.status(configured, Instant::now()).unlocked {
            Ok(owner)
        } else {
            Err("GUI is locked; enter the six-digit PIN".to_owned())
        }
    }

    async fn owner_epoch(&self) -> Result<u64, String> {
        Ok(self.authorized_owner().await?.session.epoch())
    }

    async fn same_owner(&self, epoch: u64) -> Result<tokio::sync::MutexGuard<'_, OwnerState>, String> {
        let owner = self.authorized_owner().await?;
        if owner.session.epoch() == epoch {
            Ok(owner)
        } else {
            Err("The vault was locked; start the operation again".to_owned())
        }
    }
}

const DEFAULT_WINDOW_SIZE: (f64, f64) = (720.0, 820.0);
const MIN_WINDOW_SIZE: (f64, f64) = (460.0, 600.0);
const WINDOW_SIZE_FILE: &str = "window-size.json";

fn constrained_window_size(saved: (f64, f64), available: (f64, f64)) -> (f64, f64) {
    fn dimension(saved: f64, default: f64, minimum: f64, available: f64) -> f64 {
        let requested = if saved.is_finite() && saved > 0.0 {
            saved
        } else {
            default
        };
        requested.max(minimum).min(available)
    }
    (
        dimension(
            saved.0,
            DEFAULT_WINDOW_SIZE.0,
            MIN_WINDOW_SIZE.0,
            available.0,
        ),
        dimension(
            saved.1,
            DEFAULT_WINDOW_SIZE.1,
            MIN_WINDOW_SIZE.1,
            available.1,
        ),
    )
}

fn restore_window_size(window: &WebviewWindow, data_dir: &Path) -> Result<()> {
    let saved = std::fs::read(data_dir.join(WINDOW_SIZE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(DEFAULT_WINDOW_SIZE);
    let available = window
        .current_monitor()?
        .map_or((f64::MAX, f64::MAX), |monitor| {
            let work_area = monitor
                .work_area()
                .size
                .to_logical::<f64>(monitor.scale_factor());
            (work_area.width, work_area.height)
        });
    let (width, height) = constrained_window_size(saved, available);
    window.set_min_size(Some(LogicalSize::new(
        MIN_WINDOW_SIZE.0.min(available.0),
        MIN_WINDOW_SIZE.1.min(available.1),
    )))?;
    window.set_size(LogicalSize::new(width, height))?;
    window.center()?;
    Ok(())
}

fn remember_window_size(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) else {
        return;
    };
    let size = size.to_logical::<f64>(scale);
    if size.width <= 0.0 || size.height <= 0.0 {
        return;
    }
    let path = app
        .state::<AppRuntime>()
        .vault
        .data_dir()
        .join(WINDOW_SIZE_FILE);
    if let Ok(bytes) = serde_json::to_vec(&(size.width, size.height)) {
        use std::io::Write;
        let _ = (|| -> std::io::Result<()> {
            let mut file = atomic_write_file::AtomicWriteFile::open(path)?;
            file.write_all(&bytes)?;
            file.commit()
        })();
    }
}

async fn owner_lock_state(runtime: &AppRuntime) -> Result<OwnerLockState, String> {
    let (owner, configured) = runtime.owner_state().await?;
    Ok(owner.session.status(configured, Instant::now()))
}

async fn require_owner_unlocked(runtime: &AppRuntime) -> Result<(), String> {
    runtime.authorized_owner().await.map(drop)
}

fn command_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn emit_changed(app: &AppHandle) {
    let _ = app.emit("state-changed", ());
}

#[tauri::command]
async fn get_state(runtime: State<'_, AppRuntime>) -> Result<AppState, String> {
    runtime.browser.sync().await;
    let browser = runtime.browser.status().await;
    let (owner, configured) = runtime.owner_state().await?;
    let unlocked = owner.session.status(configured, Instant::now()).unlocked;
    runtime.vault.app_state(&runtime.executable, browser, unlocked).map_err(command_error)
}

#[tauri::command]
async fn owner_status(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_touch(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    let (mut owner, configured) = runtime.owner_state().await?;
    let now = Instant::now();
    owner.session.touch(now);
    Ok(owner.session.status(configured, now))
}

#[tauri::command]
async fn owner_set_pin(
    runtime: State<'_, AppRuntime>,
    pin: String,
) -> Result<OwnerLockState, String> {
    let pin = zeroize::Zeroizing::new(pin);
    let mut owner = runtime.owner.lock().await;
    runtime.vault.set_owner_pin(&pin).map_err(command_error)?;
    let now = Instant::now();
    owner.session.unlock(now);
    Ok(owner.session.status(true, now))
}

#[tauri::command]
async fn owner_disable_pin(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    let mut owner = runtime.authorized_owner().await?;
    runtime.vault.disable_owner_pin().map_err(command_error)?;
    owner.lock(&runtime);
    Ok(owner.session.status(false, Instant::now()))
}

#[tauri::command]
async fn owner_unlock(
    runtime: State<'_, AppRuntime>,
    pin: String,
) -> Result<OwnerLockState, String> {
    let pin = zeroize::Zeroizing::new(pin);
    let (mut owner, configured) = runtime.owner_state().await?;
    if let Some(seconds) = owner.session.retry_seconds(Instant::now()) {
        return Err(format!("Too many incorrect PIN attempts; wait {seconds}s"));
    }
    if !runtime.vault.verify_owner_pin(&pin).map_err(command_error)? {
        return Err(match owner.session.reject_pin(Instant::now()) {
            Some(seconds) => format!("Incorrect PIN; wait {seconds}s before trying again"),
            None => "Incorrect PIN".to_owned(),
        });
    }
    let now = Instant::now();
    owner.session.unlock(now);
    Ok(owner.session.status(configured, now))
}

#[tauri::command]
async fn owner_lock(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    let (mut owner, configured) = runtime.owner_state().await?;
    owner.lock(&runtime);
    Ok(owner.session.status(configured, Instant::now()))
}

#[tauri::command]
async fn owner_secret_view(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
) -> Result<OwnerSecretView, String> {
    let _owner = runtime.authorized_owner().await?;
    runtime.vault.owner_secret_view(id).map_err(command_error)
}

#[tauri::command]
async fn owner_editor_drafts(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<OwnerEditorDraft>, String> {
    let _owner = runtime.authorized_owner().await?;
    runtime.vault.list_editor_drafts().map_err(command_error)
}

#[tauri::command]
async fn save_editor_draft(
    runtime: State<'_, AppRuntime>,
    draft_id: Option<Uuid>,
    mut input: ConnectionInput,
) -> Result<OwnerEditorDraft, String> {
    let _owner = runtime.authorized_owner().await?;
    input.private_key_import_path.clear();
    runtime
        .vault
        .save_editor_draft(draft_id, input)
        .map_err(command_error)
}

#[tauri::command]
async fn delete_editor_draft(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime.vault.delete_editor_draft(id).map_err(command_error)
}

#[tauri::command]
async fn copy_owner_value(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    value: String,
    sensitive: Option<bool>,
) -> Result<(), String> {
    let mut owner = runtime.authorized_owner().await?;
    owner.session.touch(Instant::now());
    let value = zeroize::Zeroizing::new(value);
    runtime.clipboard.write(&app, &value, sensitive.unwrap_or(true))
}

#[tauri::command]
async fn preview_totp_uri(
    runtime: State<'_, AppRuntime>,
    uri: String,
) -> Result<crate::browser::TotpUriPreview, String> {
    let _owner = runtime.authorized_owner().await?;
    let uri = zeroize::Zeroizing::new(uri);
    crate::browser::preview_totp_uri(&uri).map_err(|_| "invalid_totp_uri".to_owned())
}

#[tauri::command]
fn api_service_catalog() -> Vec<crate::api_catalog::ApiService> {
    crate::api_catalog::service_catalog()
}

#[tauri::command]
fn oci_regions() -> &'static [&'static str] {
    crate::oci::REGIONS
}

#[tauri::command]
async fn stage_oci_key(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    source: Option<OciKeySource>,
    text: Option<String>,
) -> Result<Option<Uuid>, String> {
    use std::io::Read;
    let mut text = zeroize::Zeroizing::new(text);
    let epoch = runtime.owner_epoch().await?;
    let source = source.unwrap_or_default();
    if !matches!(source, OciKeySource::Input) && text.is_some() {
        return Err("Key text is only accepted for manual input".to_owned());
    }
    let pem = match source {
        OciKeySource::File => {
            let Some(file) = app.dialog().file().blocking_pick_file() else {
                return Ok(None);
            };
            let path = file.into_path().map_err(command_error)?;
            let mut pem = zeroize::Zeroizing::new(String::new());
            std::fs::File::open(path)
                .map_err(command_error)?
                .take(64 * 1024 + 1)
                .read_to_string(&mut pem)
                .map_err(|_| "Unable to read OCI signing key file".to_owned())?;
            pem
        }
        OciKeySource::Clipboard => zeroize::Zeroizing::new(
            app.clipboard()
                .read_text()
                .map_err(|_| "Unable to read OCI signing key from clipboard".to_owned())?,
        ),
        OciKeySource::Input => zeroize::Zeroizing::new(
            text.take()
                .ok_or_else(|| "Enter the OCI signing key".to_owned())?,
        ),
    };
    let mut owner = runtime.same_owner(epoch).await?;
    replace_pending_oci_key(&mut owner.pending_oci_key, pem).map(Some)
}

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
enum OciKeySource {
    #[default]
    File,
    Clipboard,
    Input,
}

fn replace_pending_oci_key(
    pending: &mut Option<(Uuid, zeroize::Zeroizing<String>)>,
    pem: zeroize::Zeroizing<String>,
) -> Result<Uuid, String> {
    crate::oci::validate_pem(&pem).map_err(command_error)?;
    let id = Uuid::new_v4();
    *pending = Some((id, pem));
    Ok(id)
}

#[tauri::command]
async fn discard_oci_key(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    let mut owner = runtime.owner.lock().await;
    let pending = &mut owner.pending_oci_key;
    if pending.as_ref().is_some_and(|(key_id, _)| *key_id == id) {
        pending.take();
    }
    Ok(())
}

#[tauri::command]
async fn ssh_host_trust(runtime: State<'_, AppRuntime>) -> Result<Vec<crate::model::SshHostTrust>, String> {
    require_owner_unlocked(&runtime).await?;
    runtime.vault.ssh_host_trust().map_err(command_error)
}

#[tauri::command]
async fn set_ssh_host_trust(runtime: State<'_, AppRuntime>, id: Uuid, host: String, port: u16, fingerprint: Option<String>) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime.vault.set_ssh_host_trust(id, &host, port, fingerprint.as_deref()).map_err(command_error)
}

#[tauri::command]
async fn probe_ssh_host(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<crate::ssh_transport::SshHostProbe, String> {
    require_owner_unlocked(&runtime).await?;
    crate::ssh_transport::probe(&runtime.vault, id).await.map_err(command_error)
}

#[tauri::command]
async fn certificate_capabilities(
    runtime: State<'_, AppRuntime>,
) -> Result<crate::vault::CertificateCapabilities, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .certificate_capabilities()
        .map_err(command_error)
}

#[tauri::command]
async fn set_certificate_authority(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    binding: Option<crate::model::CertificateAuthorityBinding>,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .set_certificate_authority(id, binding)
        .map_err(command_error)
}

#[tauri::command]
async fn certificate_permissions(
    runtime: State<'_, AppRuntime>,
    enabled: Option<bool>,
    id: Option<Uuid>,
    kinds: Option<Vec<crate::certificates::CertificateKind>>,
    max_ttl_seconds: Option<u64>,
) -> Result<crate::vault::CertificateCapabilities, String> {
    require_owner_unlocked(&runtime).await?;
    use crate::vault::SigningPermissionChange;
    let change = match (enabled, id, kinds, max_ttl_seconds) {
        (Some(value), None, None, None) => SigningPermissionChange::Enable(value),
        (None, Some(item_id), Some(kinds), Some(max_ttl_seconds)) => {
            SigningPermissionChange::Allow {
                item_id,
                kinds,
                max_ttl_seconds,
            }
        }
        (None, Some(item_id), None, None) => SigningPermissionChange::Remove(item_id),
        _ => return Err("Choose one signing permission operation".into()),
    };
    runtime
        .vault
        .change_signing_permissions(change)
        .map_err(command_error)
}

#[tauri::command]
async fn certificate_issue(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    request_id: Uuid,
    request: crate::certificates::CertificateRequest,
    ttl_seconds: u64,
) -> Result<crate::certificates::IssuedCertificate, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .issue_certificate(
            crate::vault::CertificateActor::Owner,
            id,
            request_id,
            request,
            ttl_seconds,
        )
        .map_err(command_error)
}

#[tauri::command]
async fn certificate_verify(
    runtime: State<'_, AppRuntime>,
    request: crate::certificates::VerifyRequest,
) -> Result<crate::certificates::VerificationReport, String> {
    require_owner_unlocked(&runtime).await?;
    crate::certificates::verify_certificate(&request, chrono::Utc::now().timestamp())
        .map_err(command_error)
}

#[tauri::command]
async fn certificate_key_check(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    key_module_id: Option<Uuid>,
    public_or_certificate: String,
) -> Result<crate::certificates::KeyMatchReport, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .check_item_key(id, key_module_id, &public_or_certificate)
        .map_err(command_error)
}

#[tauri::command]
async fn set_ssh_host_ca_trust(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    host: String,
    port: u16,
    fingerprint: Option<String>,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .set_ssh_host_ca_trust(id, &host, port, fingerprint.as_deref())
        .map_err(command_error)
}

#[tauri::command]
async fn save_connection(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    mut input: ConnectionInput,
    expected_updated_at: Option<String>,
    pending_oci_key_id: Option<Uuid>,
    pending_oci_key_module_id: Option<Uuid>,
) -> Result<PublicConnection, String> {
    let mut owner = runtime.authorized_owner().await?;
    if let Some(id) = pending_oci_key_id {
        let pem = owner.pending_oci_key
            .as_ref()
            .filter(|(candidate, _)| *candidate == id)
            .map(|(_, pem)| pem.as_str())
            .ok_or_else(|| "Please import the RSA signing key again".to_owned())?;
        let module_id = pending_oci_key_module_id
            .ok_or_else(|| "Signing key target module is required".to_owned())?;
        let module = input
            .modules
            .iter()
            .find(|module| module.id == module_id)
            .ok_or_else(|| "Signing key module is unavailable".to_owned())?;
        let name = module
            .secret_name()
            .ok_or_else(|| "Signing key must bind a secret module".to_owned())?;
        if name == "privateKey" {
            input.secrets.private_key = Some(pem.to_owned());
        } else {
            input
                .secrets
                .named_secrets
                .insert(name.to_owned(), pem.to_owned());
        }
    }
    let private_key = staged_ssh_key(&mut input, owner.pending_private_key.as_ref())?;
    let connection = runtime
        .vault
        .save_connection_with_private_key(input, expected_updated_at.as_deref(), private_key)
        .map_err(command_error)?;
    if private_key.is_some() {
        owner.pending_private_key.take();
    }
    if pending_oci_key_id.is_some() {
        owner.pending_oci_key.take();
    }
    drop(owner);
    emit_changed(&app);
    Ok(connection)
}

fn staged_ssh_key<'a>(
    input: &mut ConnectionInput,
    pending: Option<&'a PendingPrivateKey>,
) -> Result<Option<(&'a str, &'a str)>, String> {
    if input.private_key_import_path.is_empty() {
        return Ok(None);
    }
    let key = pending
        .filter(|key| key.id.to_string() == input.private_key_import_path)
        .ok_or_else(|| "Please add the SSH private key again".to_owned())?;
    decode_private_key(&key.text, input.secrets.passphrase.as_deref()).map_err(command_error)?;
    input.private_key_import_path.clear();
    Ok(Some((key.text.as_str(), key.name.as_str())))
}

#[tauri::command]
async fn reorder_connections(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    ids: Vec<Uuid>,
    expected_order: Vec<Uuid>,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .reorder_connections(&ids, &expected_order)
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn set_connection_enabled(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    enabled: bool,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .set_connection_enabled(id, enabled)
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn delete_connection(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    expected_updated_at: String,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .trash_connection(id, Some(&expected_updated_at))
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn owner_trash(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<crate::model::PublicTrashItem>, String> {
    require_owner_unlocked(&runtime).await?;
    runtime.vault.trash_items().map_err(command_error)
}

#[tauri::command]
async fn owner_restore_item(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    deleted_at: String,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .restore_connection(id, &deleted_at)
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn owner_purge_item(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    deleted_at: String,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .purge_connection(id, &deleted_at)
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn owner_password_history(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
) -> Result<serde_json::Value, String> {
    require_owner_unlocked(&runtime).await?;
    let item = runtime.vault.get_connection(id).map_err(command_error)?;
    Ok(
        serde_json::json!({"updatedAt":item.stored.updated_at,"entries":item.secrets.password_history}),
    )
}

#[tauri::command]
async fn owner_clear_password_history(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    expected_updated_at: String,
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .vault
        .clear_password_history(id, &expected_updated_at)
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn test_connection(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
) -> Result<String, String> {
    let started = Instant::now();
    let connection = {
        let _owner = runtime.authorized_owner().await?;
        runtime.vault.get_connection(id).map_err(command_error)?
    };
    let connection_name = connection.stored.name.clone();
    let result: anyhow::Result<String> =
        if connection.stored.test_target(&connection.secrets).is_some() {
            executor::test_connection(&runtime.vault, &connection).await
        } else {
            Ok(format!(
                "Secure entry ready · {} fields",
                connection
                    .secrets
                    .available_fields(connection.stored.secret.as_ref())
                    .len()
            ))
        }
        .map_err(|error| {
            anyhow::anyhow!(crate::policy::redact(
                error.to_string(),
                &connection.stored,
                &connection.secrets,
            ))
        });
    let (status, error) = match &result {
        Ok(_) => ("success", String::new()),
        Err(error) => ("error", error.to_string()),
    };
    let _ = runtime.vault.add_activity(NewActivity {
        status: status.to_owned(),
        source: "Apply".to_owned(),
        connection_name,
        action: "Test connection".to_owned(),
        duration_ms: started.elapsed().as_millis() as u64,
        error,
    });
    emit_changed(&app);
    result.map_err(command_error)
}

#[tauri::command]
async fn update_settings(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    patch: SettingsPatch,
) -> Result<crate::model::Settings, String> {
    let settings = runtime
        .vault
        .update_settings_patch(patch)
        .map_err(command_error)?;
    runtime.browser.sync().await;
    emit_changed(&app);
    Ok(settings)
}

#[tauri::command]
fn system_integration_status(
    runtime: State<'_, AppRuntime>,
) -> Result<crate::system_integration::SystemIntegrationState, String> {
    crate::system_integration::status(PathBuf::from(&runtime.executable).as_path())
        .map_err(command_error)
}

#[tauri::command]
fn set_desktop_shortcut(
    runtime: State<'_, AppRuntime>,
    enabled: bool,
) -> Result<crate::system_integration::SystemIntegrationState, String> {
    crate::system_integration::set_desktop_shortcut(
        PathBuf::from(&runtime.executable).as_path(),
        enabled,
    )
    .map_err(command_error)
}

#[tauri::command]
fn set_launch_at_login(
    runtime: State<'_, AppRuntime>,
    enabled: bool,
) -> Result<crate::system_integration::SystemIntegrationState, String> {
    crate::system_integration::set_launch_at_login(
        PathBuf::from(&runtime.executable).as_path(),
        enabled,
    )
    .map_err(command_error)
}

#[tauri::command]
async fn clear_activities(app: AppHandle, runtime: State<'_, AppRuntime>) -> Result<(), String> {
    runtime.vault.clear_activities().map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn copy_mcp_config(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    format: String,
) -> Result<String, String> {
    let config = mcp::render_config(&format).map_err(command_error)?;
    runtime.clipboard.write(&app, &config, false)?;
    Ok(config)
}

#[tauri::command]
async fn agent_mcp_status(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<AgentClientStatus>, String> {
    Ok(runtime.agent_registry.list().await)
}

#[tauri::command]
async fn agent_mcp_register(
    runtime: State<'_, AppRuntime>,
    client_ids: Vec<String>,
) -> Result<Vec<AgentActionResult>, String> {
    Ok(runtime.agent_registry.register(&client_ids).await)
}

#[tauri::command]
async fn agent_mcp_repair(
    runtime: State<'_, AppRuntime>,
    client_id: String,
) -> Result<AgentActionResult, String> {
    Ok(runtime.agent_registry.repair(&client_id).await)
}

#[tauri::command]
async fn agent_mcp_remove(
    runtime: State<'_, AppRuntime>,
    client_id: String,
) -> Result<AgentActionResult, String> {
    Ok(runtime.agent_registry.remove(&client_id).await)
}

struct PendingPrivateKey {
    id: Uuid,
    name: String,
    text: zeroize::Zeroizing<String>,
    public_key: String,
    fingerprint: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PrivateKeyPreview {
    id: Uuid,
    name: String,
    fingerprint: String,
}

fn decode_private_key(text: &str, passphrase: Option<&str>) -> Result<russh::keys::PrivateKey> {
    crate::ssh_keys::decode_private_key(text, passphrase)
}

impl PendingPrivateKey {
    fn parse(name: String, text: String, passphrase: Option<&str>) -> Result<Self> {
        let text = zeroize::Zeroizing::new(text);
        let key = decode_private_key(&text, passphrase)?;
        Ok(Self {
            id: Uuid::new_v4(),
            name,
            text,
            public_key: key.public_key().to_openssh()?,
            fingerprint: key
                .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
                .to_string(),
        })
    }

    fn generate() -> Result<Self> {
        use russh::keys::ssh_key::{LineEnding, private::Ed25519Keypair};
        let mut seed = zeroize::Zeroizing::new([0_u8; 32]);
        getrandom::fill(&mut *seed)
            .map_err(|error| anyhow::anyhow!("Could not generate SSH key: {error}"))?;
        let key = russh::keys::PrivateKey::from(Ed25519Keypair::from_seed(&seed));
        Self::parse(
            "Ed25519".into(),
            key.to_openssh(LineEnding::LF)?.to_string(),
            None,
        )
    }
}

#[tauri::command]
async fn stage_private_key(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    source: String,
    passphrase: Option<String>,
) -> Result<Option<PrivateKeyPreview>, String> {
    let epoch = runtime.owner_epoch().await?;
    let passphrase = zeroize::Zeroizing::new(passphrase);
    let pending = match source.as_str() {
        "generate" => PendingPrivateKey::generate(),
        "file" => {
            let selected = app.dialog().file().blocking_pick_file();
            let Some(path) = selected else {
                return Ok(None);
            };
            let path = path.into_path().map_err(command_error)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("SSH key");
            let text = std::fs::read_to_string(&path).map_err(command_error)?;
            PendingPrivateKey::parse(name.to_owned(), text, passphrase.as_deref())
        }
        "clipboard" => {
            let text = app.clipboard().read_text().map_err(command_error)?;
            PendingPrivateKey::parse("Clipboard".into(), text, passphrase.as_deref())
        }
        _ => return Err("Unknown SSH key source".into()),
    }
    .map_err(command_error)?;
    let mut owner = runtime.same_owner(epoch).await?;
    let preview = PrivateKeyPreview {
        id: pending.id,
        name: pending.name.clone(),
        fingerprint: pending.fingerprint.clone(),
    };
    owner.pending_private_key = Some(pending);
    Ok(Some(preview))
}

#[tauri::command]
async fn discard_private_key(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    let mut owner = runtime.owner.lock().await;
    let pending = &mut owner.pending_private_key;
    if pending.as_ref().is_some_and(|key| key.id == id) {
        pending.take();
    }
    Ok(())
}

#[tauri::command]
async fn copy_ssh_public_key(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    pending_id: Option<Uuid>,
    item_id: Option<Uuid>,
) -> Result<(), String> {
    let owner = runtime.authorized_owner().await?;
    let public_key = if let Some(id) = pending_id {
        owner
            .pending_private_key
            .as_ref()
            .filter(|key| key.id == id)
            .map(|key| key.public_key.clone())
            .ok_or_else(|| "Please add the SSH private key again".to_owned())?
    } else {
        let id =
            item_id.ok_or_else(|| "Save the SSH key before copying its public key".to_owned())?;
        let item = runtime.vault.get_connection(id).map_err(command_error)?;
        let text = item
            .secrets
            .private_key
            .as_deref()
            .ok_or_else(|| "No SSH private key in this item".to_owned())?;
        decode_private_key(text, item.secrets.passphrase.as_deref())
            .map_err(command_error)?
            .public_key()
            .to_openssh()
            .map_err(command_error)?
    };
    runtime.clipboard.write(&app, &public_key, false)
}

#[tauri::command]
async fn quick_pair_browser(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    port: u16,
) -> Result<(), String> {
    runtime
        .vault
        .update_settings_patch(SettingsPatch {
            browser_enabled: Some(true),
            browser_port: Some(port),
            ..SettingsPatch::default()
        })
        .map_err(command_error)?;
    runtime.browser.sync().await;
    runtime
        .browser
        .start_quick_pairing()
        .await
        .map_err(command_error)?;

    emit_changed(&app);

    Ok(())
}

#[tauri::command]
async fn reset_browser_pairing(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
) -> Result<(), String> {
    runtime
        .browser
        .reset_pairing()
        .await
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(())
}

#[tauri::command]
async fn open_browser_extension_folder(app: AppHandle) -> Result<(), String> {
    let path = browser_extension_path(&app)?;
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(command_error)
}

#[tauri::command]
async fn drag_browser_extension(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    let path = browser_extension_path(&app)?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let drag_window = window.clone();
    window
        .run_on_main_thread(move || {
            let result = (|| -> Result<(), String> {
                #[cfg(target_os = "linux")]
                let native = drag_window.gtk_window().map_err(command_error)?;
                #[cfg(target_os = "linux")]
                let handle = &native;
                #[cfg(not(target_os = "linux"))]
                let handle = &drag_window;
                drag::start_drag(
                    handle,
                    drag::DragItem::Files(vec![path]),
                    drag::Image::Raw(include_bytes!("../icons/32x32.png").to_vec()),
                    |_, _| {},
                    drag::Options::default(),
                )
                .map_err(command_error)
            })();
            let _ = sender.send(result);
        })
        .map_err(command_error)?;
    receiver.await.map_err(command_error)?
}

fn browser_extension_path(app: &AppHandle) -> Result<PathBuf, String> {
    let bundled = app
        .path()
        .resource_dir()
        .ok()
        .map(|path| path.join("browser-extension"))
        .filter(|path| path.is_dir());
    let executable = mcp::launcher_executable().map_err(command_error)?;
    let portable = executable
        .parent()
        .map(|path| path.join("browser-extension"))
        .filter(|path| path.is_dir());
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|path| path.join("browser-extension"))
        .filter(|path| path.is_dir());
    let path = portable.or(bundled).or(development).ok_or_else(|| {
        "browser-extension directory not found; use the complete portable ZIP".to_owned()
    })?;
    Ok(path)
}

#[tauri::command]
fn open_chromium_browser() -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut candidates = Vec::new();
        for root in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
            if let Some(root) = std::env::var_os(root) {
                let root = PathBuf::from(root);
                candidates.push(root.join("Google/Chrome/Application/chrome.exe"));
                candidates.push(root.join("Microsoft/Edge/Application/msedge.exe"));
                candidates.push(root.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
            }
        }
        candidates.extend(["chrome.exe", "msedge.exe", "brave.exe"].map(PathBuf::from));
        if candidates.into_iter().any(|candidate| {
            (candidate.components().count() == 1 || candidate.is_file())
                && Command::new(candidate).arg("--new-tab").spawn().is_ok()
        }) {
            return Ok(());
        }
    }
    #[cfg(target_os = "macos")]
    {
        if ["Google Chrome", "Microsoft Edge", "Brave Browser"]
            .into_iter()
            .any(|browser| {
                Command::new("open")
                    .args(["-a", browser])
                    .status()
                    .is_ok_and(|status| status.success())
            })
        {
            return Ok(());
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if [
            "google-chrome",
            "google-chrome-stable",
            "microsoft-edge",
            "brave-browser",
            "chromium",
            "chromium-browser",
        ]
        .into_iter()
        .any(|browser| Command::new(browser).arg("--new-tab").spawn().is_ok())
        {
            return Ok(());
        }
    }
    Err(
        "Unable to open a Chromium-based browser; open one manually and paste the copied address"
            .to_owned(),
    )
}

#[tauri::command]
async fn generate_password(
    runtime: State<'_, AppRuntime>,
    options: crate::generator::Options,
) -> Result<String, String> {
    require_owner_unlocked(&runtime).await?;
    crate::generator::password(&options).map_err(command_error)
}

#[tauri::command]
async fn export_backup(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    password: String,
) -> Result<Option<String>, String> {
    let epoch = runtime.owner_epoch().await?;
    let password = zeroize::Zeroizing::new(password);
    let selected = app
        .dialog()
        .file()
        .add_filter("OpenKeylet backup", &["mvault"])
        .set_file_name("openkeylet-backup.mvault")
        .blocking_save_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    let _owner = runtime.same_owner(epoch).await?;
    let path = path.into_path().map_err(command_error)?;
    backup::export_protected(&runtime.vault, &path, &password).map_err(command_error)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn github_backup_status(runtime: State<'_, AppRuntime>) -> Result<BackupStatus, String> {
    let epoch = runtime.owner_epoch().await?;
    let backup = runtime.github_backup.lock().await;
    let _owner = runtime.same_owner(epoch).await?;
    backup.status().map_err(command_error)
}

#[tauri::command]
async fn github_backup_account(
    runtime: State<'_, AppRuntime>,
    input: github_backup::AuthInput,
) -> Result<github_backup::AccountStatus, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .account(input)
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn github_backup_discover(
    runtime: State<'_, AppRuntime>,
    input: github_backup::AuthInput,
) -> Result<github_backup::DiscoveryResult, String> {
    let epoch = runtime.owner_epoch().await?;
    let backup = runtime.github_backup.lock().await;
    drop(runtime.same_owner(epoch).await?);
    let result = backup.discover(input).await;
    let _owner = runtime.same_owner(epoch).await?;
    result.map_err(command_error)
}

#[tauri::command]
async fn github_backup_generate_key(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    require_owner_unlocked(&runtime).await?;
    github_backup::generate_recovery_key().map_err(command_error)
}

#[tauri::command]
async fn github_backup_recovery_key(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    let epoch = runtime.owner_epoch().await?;
    let backup = runtime.github_backup.lock().await;
    let _owner = runtime.same_owner(epoch).await?;
    backup.recovery_key().map_err(command_error)
}

#[tauri::command]
async fn github_backup_connect(
    runtime: State<'_, AppRuntime>,
    input: ConnectInput,
) -> Result<BackupStatus, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .connect(input)
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn github_backup_upload(runtime: State<'_, AppRuntime>) -> Result<BackupStatus, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .upload()
        .await
        .map_err(command_error)
}

#[tauri::command]
async fn github_backup_preview_restore(
    runtime: State<'_, AppRuntime>,
) -> Result<github_backup::RestorePreview, String> {
    let epoch = runtime.owner_epoch().await?;
    let backup = runtime.github_backup.lock().await;
    drop(runtime.same_owner(epoch).await?);
    let result = backup.preview_restore().await;
    let _owner = runtime.same_owner(epoch).await?;
    result.map_err(command_error)
}

#[tauri::command]
async fn github_backup_restore(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    snapshot: github_backup::RestoreSnapshot,
) -> Result<RestoreResult, String> {
    require_owner_unlocked(&runtime).await?;
    let result = runtime
        .github_backup
        .lock()
        .await
        .restore(Some(snapshot))
        .await
        .map_err(command_error)?;
    emit_changed(&app);
    Ok(result)
}

#[tauri::command]
async fn github_backup_disconnect(runtime: State<'_, AppRuntime>) -> Result<BackupStatus, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .disconnect()
        .map_err(command_error)
}

#[tauri::command]
async fn github_backup_help(app: AppHandle, page: String) -> Result<(), String> {
    let url = match page.as_str() {
        "repository" => "https://github.com/new",
        "token" => "https://github.com/settings/personal-access-tokens/new",
        "login" => "https://cli.github.com/manual/gh_auth_login",
        _ => return Err("GITHUB_HELP_INVALID".to_owned()),
    };
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(command_error)
}

#[tauri::command]
async fn prepare_backup_restore(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    password: String,
    id: Uuid,
) -> Result<Option<serde_json::Value>, String> {
    let epoch = {
        let mut owner = runtime.authorized_owner().await?;
        owner.pending_backup = Some(SelectableImportState::Preparing(id));
        owner.session.epoch()
    };
    let password = zeroize::Zeroizing::new(password);
    let selected = app
        .dialog()
        .file()
        .add_filter("OpenKeylet backup", &["mvault"])
        .blocking_pick_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    drop(runtime.same_owner(epoch).await?);
    let path = path.into_path().map_err(command_error)?;
    let prepared = tauri::async_runtime::spawn_blocking(move || backup::prepare_restore(&path, &password))
        .await
        .map_err(command_error)?
        .map_err(command_error)?;
    let mut owner = runtime.same_owner(epoch).await?;
    if !matches!(owner.pending_backup, Some(SelectableImportState::Preparing(current)) if current == id) {
        return Err("Backup preview changed; choose the file again".into());
    }
    let preview = prepared.preview(&runtime.vault).map_err(command_error)?;
    owner.pending_backup = Some(SelectableImportState::Ready(PendingImport {
        id: id.to_string(),
        created: Instant::now(),
        encrypted: runtime.vault.protect_local_state(&prepared).map_err(command_error)?,
    }));
    Ok(Some(serde_json::json!({ "id": id, "preview": preview })))
}

#[tauri::command]
async fn commit_backup_restore(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: Uuid,
    selection: backup::RestoreSelection,
) -> Result<ImportSummary, String> {
    let mut owner = runtime.authorized_owner().await?;
    let slot = &mut owner.pending_backup;
    let Some(SelectableImportState::Ready(pending)) = slot.as_ref().filter(|state| state.matches(id)) else {
        return Err("Backup preview changed; choose the file again".into());
    };
    if pending.created.elapsed().as_secs() >= 300 {
        *slot = None;
        return Err("Backup preview expired; choose the file again".into());
    }
    let prepared: backup::PreparedBackup = runtime.vault
        .unprotect_local_state(&pending.encrypted).map_err(command_error)?;
    let summary = prepared.restore_selected(&runtime.vault, selection).map_err(command_error)?;
    *slot = None;
    emit_changed(&app);
    Ok(summary)
}

#[tauri::command]
async fn cancel_backup_restore(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    let mut owner = runtime.owner.lock().await;
    if owner.pending_backup.as_ref().is_some_and(|pending| pending.matches(id)) {
        owner.pending_backup = None;
    }
    Ok(())
}

#[tauri::command]
async fn prepare_import(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    format: String,
) -> Result<Option<serde_json::Value>, String> {
    let epoch = {
        let mut owner = runtime.authorized_owner().await?;
        owner.pending_import = None;
        owner.session.epoch()
    };
    let extension = match format.as_str() {
        "bitwarden" => "json",
        "chrome" => "csv",
        _ => return Err("Unsupported import format".into()),
    };
    let selected = app
        .dialog()
        .file()
        .add_filter("Vault export", &[extension])
        .blocking_pick_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    let mut owner = runtime.same_owner(epoch).await?;
    let parsed = crate::migration::read(&path.into_path().map_err(command_error)?, &format)
        .map_err(command_error)?;
    stage_import(&runtime.vault, &mut owner, &parsed.entries, parsed.preview).map(Some)
}

#[tauri::command]
async fn prepare_curl_import(
    runtime: State<'_, AppRuntime>,
    command: String,
) -> Result<serde_json::Value, String> {
    let command = zeroize::Zeroizing::new(command);
    let mut owner = runtime.authorized_owner().await?;
    owner.pending_import = None;
    let (entry, preview) = crate::migration::parse_curl(&command).map_err(command_error)?;
    stage_import(&runtime.vault, &mut owner, &[entry], preview)
}

fn stage_import(
    vault: &Vault,
    owner: &mut OwnerState,
    entries: &[(PortableConnection, SecretBundle)],
    preview: impl serde::Serialize,
) -> Result<serde_json::Value, String> {
    let id = Uuid::new_v4().to_string();
    let encrypted = vault.protect_local_state(&entries).map_err(command_error)?;
    let result = serde_json::json!({"id":id,"preview":preview});
    owner.pending_import = Some(PendingImport {
        id,
        created: Instant::now(),
        encrypted,
    });
    Ok(result)
}

#[tauri::command]
async fn commit_import(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<ImportSummary, String> {
    let mut owner = runtime.authorized_owner().await?;
    let slot = &mut owner.pending_import;
    let pending = slot.as_ref().ok_or("Prepare an import preview first")?;
    if pending.id != id {
        return Err("Import preview changed; prepare a new preview".into());
    }
    if pending.created.elapsed().as_secs() > 300 {
        *slot = None;
        return Err("Import preview expired; prepare a new preview".into());
    }
    let entries = runtime
        .vault
        .unprotect_local_state(&pending.encrypted)
        .map_err(command_error)?;
    let summary = runtime
        .vault
        .merge_connections(entries)
        .map_err(command_error)?;
    *slot = None;
    emit_changed(&app);
    Ok(summary)
}

#[tauri::command]
async fn cancel_import(runtime: State<'_, AppRuntime>) -> Result<(), String> {
    runtime.owner.lock().await.pending_import = None;
    Ok(())
}

#[derive(Clone, Copy)]
enum LocalImportKind {
    Env,
    Ssh,
}

#[derive(serde::Serialize, serde::Deserialize)]
enum LocalImport {
    Env(env_import::EnvImport),
    Ssh(ssh_config_import::SshConfigImport),
}

impl LocalImportKind {
    fn pending(self, owner: &mut OwnerState) -> &mut Option<SelectableImportState> {
        match self {
            Self::Env => &mut owner.pending_env,
            Self::Ssh => &mut owner.pending_ssh,
        }
    }

    fn undo(self, owner: &mut OwnerState) -> &mut Option<env_import::UndoBatch> {
        match self {
            Self::Env => &mut owner.env_undo,
            Self::Ssh => &mut owner.ssh_undo,
        }
    }

    fn read(self, path: &Path) -> Result<LocalImport> {
        Ok(match self {
            Self::Env => LocalImport::Env(env_import::EnvImport::read(path)?),
            Self::Ssh => LocalImport::Ssh(ssh_config_import::SshConfigImport::read(path)?),
        })
    }
}

impl LocalImport {
    fn preview(&self, vault: &Vault, id: Uuid) -> Result<serde_json::Value> {
        Ok(match self {
            Self::Env(import) => serde_json::to_value(import.preview(vault, id)?)?,
            Self::Ssh(import) => serde_json::to_value(import.preview(vault, id)?)?,
        })
    }

    fn commit(&self, vault: &Vault, selection: Vec<usize>) -> Result<(env_import::CommitSummary, Option<env_import::UndoBatch>)> {
        match self {
            Self::Env(import) => import.commit(vault, selection),
            Self::Ssh(import) => import.commit(vault, selection),
        }
    }
}

async fn prepare_local_import(
    app: &AppHandle,
    runtime: &AppRuntime,
    kind: LocalImportKind,
    id: Uuid,
) -> Result<Option<serde_json::Value>, String> {
    let epoch = {
        let mut owner = runtime.authorized_owner().await?;
        *kind.pending(&mut owner) = Some(SelectableImportState::Preparing(id));
        owner.session.epoch()
    };
    let result = async {
        let selected = app.dialog().file().blocking_pick_file();
        let Some(path) = selected else {
            return Ok(None);
        };
        drop(runtime.same_owner(epoch).await?);
        let path = path.into_path().map_err(command_error)?;
        let import = tauri::async_runtime::spawn_blocking(move || kind.read(&path))
            .await
            .map_err(command_error)?
            .map_err(command_error)?;
        let mut owner = runtime.same_owner(epoch).await?;
        if !matches!(kind.pending(&mut owner), Some(SelectableImportState::Preparing(current)) if *current == id) {
            return Err("Import preview changed; choose the file again".to_owned());
        }
        let preview = import.preview(&runtime.vault, id).map_err(command_error)?;
        *kind.pending(&mut owner) = Some(SelectableImportState::Ready(PendingImport {
            id: id.to_string(),
            created: Instant::now(),
            encrypted: runtime.vault.protect_local_state(&import).map_err(command_error)?,
        }));
        Ok(Some(preview))
    }
    .await;
    if !matches!(result, Ok(Some(_))) {
        cancel_local_import(runtime, kind, id).await;
    }
    result
}

async fn commit_local_import(
    app: &AppHandle,
    runtime: &AppRuntime,
    kind: LocalImportKind,
    id: Uuid,
    selection: Vec<usize>,
) -> Result<env_import::CommitSummary, String> {
    let mut owner = runtime.authorized_owner().await?;
    let slot = kind.pending(&mut owner);
    let Some(SelectableImportState::Ready(pending)) =
        slot.as_ref().filter(|state| state.matches(id))
    else {
        return Err("Import preview changed; choose the file again".into());
    };
    if pending.created.elapsed().as_secs() >= 300 {
        *slot = None;
        return Err("Import preview expired; choose the file again".into());
    }
    let import: LocalImport = runtime.vault
        .unprotect_local_state(&pending.encrypted)
        .map_err(command_error)?;
    let (summary, undo) = import.commit(&runtime.vault, selection).map_err(command_error)?;
    *slot = None;
    if undo.is_some() {
        *kind.undo(&mut owner) = undo;
        emit_changed(app);
    }
    Ok(summary)
}

async fn cancel_local_import(runtime: &AppRuntime, kind: LocalImportKind, id: Uuid) {
    let mut owner = runtime.owner.lock().await;
    let slot = kind.pending(&mut owner);
    if slot.as_ref().is_some_and(|state| state.matches(id)) {
        *slot = None;
    }
}

async fn undo_local_import(
    app: &AppHandle,
    runtime: &AppRuntime,
    kind: LocalImportKind,
    id: Uuid,
) -> Result<env_import::UndoSummary, String> {
    let mut owner = runtime.authorized_owner().await?;
    let undo = kind.undo(&mut owner);
    let receipt = undo.as_ref().filter(|receipt| receipt.id == id)
        .ok_or("This import can no longer be undone")?;
    let summary = receipt.undo(&runtime.vault).map_err(command_error)?;
    *undo = None;
    if summary.removed != 0 {
        emit_changed(app);
    }
    Ok(summary)
}

#[tauri::command]
async fn prepare_env_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid,
) -> Result<Option<serde_json::Value>, String> {
    prepare_local_import(&app, &runtime, LocalImportKind::Env, id).await
}

#[tauri::command]
async fn commit_env_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid, selection: Vec<usize>,
) -> Result<env_import::CommitSummary, String> {
    commit_local_import(&app, &runtime, LocalImportKind::Env, id, selection).await
}

#[tauri::command]
async fn cancel_env_import(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    cancel_local_import(&runtime, LocalImportKind::Env, id).await;
    Ok(())
}

#[tauri::command]
async fn undo_env_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid,
) -> Result<env_import::UndoSummary, String> {
    undo_local_import(&app, &runtime, LocalImportKind::Env, id).await
}

#[tauri::command]
async fn prepare_ssh_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid,
) -> Result<Option<serde_json::Value>, String> {
    prepare_local_import(&app, &runtime, LocalImportKind::Ssh, id).await
}

#[tauri::command]
async fn commit_ssh_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid, selection: Vec<usize>,
) -> Result<env_import::CommitSummary, String> {
    commit_local_import(&app, &runtime, LocalImportKind::Ssh, id, selection).await
}

#[tauri::command]
async fn cancel_ssh_import(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    cancel_local_import(&runtime, LocalImportKind::Ssh, id).await;
    Ok(())
}

#[tauri::command]
async fn undo_ssh_import(
    app: AppHandle, runtime: State<'_, AppRuntime>, id: Uuid,
) -> Result<env_import::UndoSummary, String> {
    undo_local_import(&app, &runtime, LocalImportKind::Ssh, id).await
}

#[tauri::command]
async fn open_data_folder(app: AppHandle, runtime: State<'_, AppRuntime>) -> Result<(), String> {
    app.opener()
        .open_path(runtime.vault.data_dir().to_string_lossy(), None::<&str>)
        .map_err(command_error)
}

#[tauri::command]
async fn window_action(
    window: WebviewWindow,
    runtime: State<'_, AppRuntime>,
    action: String,
) -> Result<(), String> {
    match action.as_str() {
        "minimize" => window.minimize().map_err(command_error),
        "close" => {
            remember_window_size(window.app_handle());
            let tray_available = runtime.tray_available.load(Ordering::Acquire);
            if !tray_available {
                window.app_handle().exit(0);
                return Ok(());
            }
            let close_behavior = runtime
                .vault
                .settings()
                .map_err(command_error)?
                .close_behavior;
            if !should_hide_on_close(&close_behavior, tray_available) {
                window.app_handle().exit(0);
                Ok(())
            } else {
                window.hide().map_err(command_error)
            }
        }
        _ => Err("Unknown window operation".to_owned()),
    }
}

fn should_hide_on_close(close_behavior: &str, tray_available: bool) -> bool {
    tray_available && !close_behavior.eq_ignore_ascii_case("exit")
}

fn show_main_window(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn install_tray(app: &tauri::App) -> Result<()> {
    let open = MenuItem::with_id(app, "tray-open", "OPEN OpenKeylet", true, None::<&str>)?;
    let lock = MenuItem::with_id(app, "tray-lock", "LOCK", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "tray-quit", "QUIT", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &lock, &separator, &quit])?;
    #[cfg(target_os = "macos")]
    let icon = app
        .default_window_icon()
        .context("OpenKeylet tray icon not found")?
        .clone();
    #[cfg(not(target_os = "macos"))]
    let icon = tauri::include_image!("icons/tray.png");

    let builder = TrayIconBuilder::with_id("openkeylet-tray")
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("OpenKeylet — LOCAL VAULT")
        .menu(&menu)
        .show_menu_on_left_click(false);
    #[cfg(target_os = "linux")]
    let builder = builder.temp_dir_path(
        app.path()
            .app_cache_dir()?
            .join("tray")
            .join(std::process::id().to_string()),
    );

    let tray = builder
        .on_menu_event(|app, event| {
            if event.id() == "tray-open" {
                show_main_window(app);
            } else if event.id() == "tray-lock" {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let runtime = handle.state::<AppRuntime>();
                    if let Ok((mut owner, configured)) = runtime.owner_state().await {
                        owner.lock(&runtime);
                        let _ = handle.emit(
                            if configured { "owner-locked" } else { "pin-setup-requested" },
                            (),
                        );
                    }
                    show_main_window(&handle);
                });
            } else if event.id() == "tray-quit" {
                if let Err(error) =
                    crate::runtime_epoch::invalidate(app.state::<AppRuntime>().vault.data_dir())
                {
                    eprintln!("OpenKeylet: failed to stop MCP sessions: {error:#}");
                }
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            _ => {}
        })
        .build(app)?;
    #[cfg(target_os = "linux")]
    {
        // GTK exports the dbusmenu tree from idle callbacks. Keep the indicator
        // passive until those callbacks finish so hosts cannot cache an empty
        // first layout and then refuse to open it to request an update.
        tray.set_visible(false)?;
        glib::idle_add_local_full(glib::Priority::LOW, move || {
            if let Err(error) = tray.set_visible(true) {
                eprintln!("OpenKeylet: failed to show the system tray: {error:#}");
            }
            glib::ControlFlow::Break
        });
    }
    #[cfg(not(target_os = "linux"))]
    let _ = tray;
    Ok(())
}

#[cfg(windows)]
fn request_rounded_corners(window: &WebviewWindow) -> Result<()> {
    use windows::Win32::Graphics::Dwm::{
        DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
    };

    let hwnd = window.hwnd().context("Unable to obtain window handle")?;
    // Match the rounded native Bricktone shell; CSS keeps inner wells at 4px.
    let preference = DWMWCP_ROUND;
    unsafe {
        DwmSetWindowAttribute(
            windows::Win32::Foundation::HWND(hwnd.0),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &preference as *const _ as *const _,
            size_of_val(&preference) as u32,
        )
        .context("Unable to request a rounded-corner window on Windows")?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn request_rounded_corners(_window: &WebviewWindow) -> Result<()> {
    Ok(())
}

pub fn run_gui() -> Result<()> {
    let context = tauri::generate_context!();
    #[cfg(target_os = "linux")]
    let context = {
        let mut context = context;
        let main = context
            .config_mut()
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "main")
            .context("Main window configuration not found")?;
        // Both the GTK surface and WebKit backing must be transparent; an
        // opaque native background fills the CSS-rounded corners back in.
        main.transparent = true;
        main.background_color = Some(tauri::window::Color(0, 0, 0, 0));
        context
    };
    let data_dir = app_data_dir()?;
    let mut gui_instance = GuiInstance::acquire(&data_dir)?;
    #[cfg(target_os = "linux")]
    if let Err(error) =
        crate::system_integration::ensure_application_entry(&mcp::launcher_executable()?)
    {
        eprintln!("OpenKeylet: failed to register the application launcher: {error:#}");
    }
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .setup(move |app| {
            let exit_handle = app.handle().clone();
            gui_instance.listen_for_takeover(move || exit_handle.exit(0))?;
            let vault = Vault::open(data_dir.clone())?;
            let executable = mcp::launcher_executable()?.to_string_lossy().into_owned();
            if let Err(error) = crate::system_integration::migrate_legacy_entries(Path::new(&executable)) {
                eprintln!("OpenKeylet: failed to migrate legacy system shortcuts: {error:#}");
            }
            let browser = BrowserBridge::new(vault.clone());
            let agent_registry = AgentRegistry::new(PathBuf::from(&executable))?;
            agent_registry.refresh_managed_instructions();
            app.manage(AppRuntime {
                _gui_instance: gui_instance,
                github_backup: Mutex::new(GithubBackup::new(vault.clone())?),
                vault,
                executable,
                browser,
                owner: Mutex::new(OwnerState::default()),
                app: app.handle().clone(),
                clipboard: ClipboardGuard::default(),
                agent_registry,
                tray_available: AtomicBool::new(false),
            });

            let owner_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tick.tick().await;
                    let runtime = owner_app.state::<AppRuntime>();
                    if runtime.owner.lock().await.session.expired(Instant::now()) {
                        let _ = owner_lock_state(&runtime).await;
                    }
                }
            });

            let window = app.get_webview_window("main").context("Main window not found")?;
            #[cfg(debug_assertions)]
            window.set_title("OpenKeylet · DEV")?;
            restore_window_size(&window, app.state::<AppRuntime>().vault.data_dir())?;
            request_rounded_corners(&window)?;
            match install_tray(app) {
                Ok(()) => app
                    .state::<AppRuntime>()
                    .tray_available
                    .store(true, Ordering::Release),
                Err(error) => {
                    eprintln!(
                        "OpenKeylet: system tray unavailable; closing the window will exit: {error:#}"
                    )
                }
            }
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    handle.state::<AppRuntime>().browser.sync().await;
                    emit_changed(&handle);
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                remember_window_size(window.app_handle());
                let runtime = window.state::<AppRuntime>();
                let close_behavior = runtime
                    .vault
                    .settings()
                    .map(|settings| settings.close_behavior)
                    .unwrap_or_else(|_| "exit".to_owned());
                if should_hide_on_close(
                    &close_behavior,
                    runtime.tray_available.load(Ordering::Acquire),
                ) {
                    api.prevent_close();
                    let _ = window.hide();
                } else if let Err(error) =
                    crate::runtime_epoch::invalidate(runtime.vault.data_dir())
                {
                    eprintln!("OpenKeylet: failed to stop MCP sessions: {error:#}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            owner_status,
            owner_touch,
            owner_set_pin,
            owner_disable_pin,
            owner_unlock,
            owner_lock,
            owner_secret_view,
            owner_editor_drafts,
            save_editor_draft,
            delete_editor_draft,
            copy_owner_value,
            preview_totp_uri,
            save_connection,
            ssh_host_trust,
            set_ssh_host_trust,
            probe_ssh_host,
            set_ssh_host_ca_trust,
            certificate_capabilities,
            set_certificate_authority,
            certificate_permissions,
            certificate_issue,
            certificate_verify,
            certificate_key_check,
            oci_regions,
            api_service_catalog,
            stage_oci_key,
            discard_oci_key,
            reorder_connections,
            set_connection_enabled,
            delete_connection,
            owner_trash,
            owner_restore_item,
            owner_purge_item,
            owner_password_history,
            owner_clear_password_history,
            test_connection,
            update_settings,
            system_integration_status,
            set_desktop_shortcut,
            set_launch_at_login,
            clear_activities,
            copy_mcp_config,
            agent_mcp_status,
            agent_mcp_register,
            agent_mcp_repair,
            agent_mcp_remove,
            stage_private_key,
            discard_private_key,
            copy_ssh_public_key,
            quick_pair_browser,
            open_chromium_browser,
            reset_browser_pairing,
            open_browser_extension_folder,
            drag_browser_extension,
            export_backup,
            generate_password,
            prepare_backup_restore,
            commit_backup_restore,
            cancel_backup_restore,
            prepare_import,
            prepare_curl_import,
            commit_import,
            cancel_import,
            prepare_env_import,
            commit_env_import,
            cancel_env_import,
            undo_env_import,
            prepare_ssh_import,
            commit_ssh_import,
            cancel_ssh_import,
            undo_ssh_import,
            github_backup_status,
            github_backup_account,
            github_backup_discover,
            github_backup_generate_key,
            github_backup_recovery_key,
            github_backup_connect,
            github_backup_upload,
            github_backup_preview_restore,
            github_backup_restore,
            github_backup_disconnect,
            github_backup_help,
            open_data_folder,
            window_action,
        ])
        .build(context)
        .context("OpenKeylet GUI startup failed")?;

    #[cfg(target_os = "macos")]
    app.run(|app, event| {
        if matches!(&event, tauri::RunEvent::ExitRequested { .. }) {
            remember_window_size(app);
        }
        if let tauri::RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
    });

    #[cfg(not(target_os = "macos"))]
    app.run(|app, event| {
        if matches!(&event, tauri::RunEvent::ExitRequested { .. }) {
            remember_window_size(app);
        }
        // A hidden development launcher can pass its initial show state to the
        // child process. Explicitly show the GUI once its event loop is ready.
        if matches!(event, tauri::RunEvent::Ready) {
            show_main_window(app);
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{constrained_window_size, should_hide_on_close};

    #[test]
    fn window_size_fits_the_current_screen() {
        assert_eq!(
            constrained_window_size((950.0, 900.0), (850.0, 740.0)),
            (850.0, 740.0)
        );
        assert_eq!(
            constrained_window_size((500.0, 650.0), (850.0, 740.0)),
            (500.0, 650.0)
        );
        assert_eq!(
            constrained_window_size((0.0, f64::NAN), (850.0, 900.0)),
            (720.0, 820.0)
        );
    }

    #[test]
    fn close_only_hides_when_a_tray_entry_exists() {
        assert!(should_hide_on_close("tray", true));
        assert!(!should_hide_on_close("tray", false));
        assert!(!should_hide_on_close("exit", true));
    }
}

#[cfg(test)]
mod oci_key_tests {
    use super::*;

    #[test]
    fn oci_key_sources_have_explicit_wire_names_and_file_default() {
        for (name, expected) in [
            ("file", OciKeySource::File),
            ("clipboard", OciKeySource::Clipboard),
            ("input", OciKeySource::Input),
        ] {
            let source: OciKeySource = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(
                std::mem::discriminant(&source),
                std::mem::discriminant(&expected)
            );
        }
        assert!(matches!(OciKeySource::default(), OciKeySource::File));
        assert!(serde_json::from_value::<OciKeySource>(serde_json::json!("generate")).is_err());
    }

    #[test]
    fn invalid_oci_input_does_not_replace_a_staged_key() {
        let mut pending = None;
        let first = replace_pending_oci_key(
            &mut pending,
            zeroize::Zeroizing::new(crate::oci::tests::KEY.into()),
        )
        .unwrap();
        let ssh = PendingPrivateKey::generate().unwrap();
        for text in [String::new(), "INVALID-OCI-INPUT-MARKER".into(), ssh.text.to_string(), "-----BEGIN ENCRYPTED PRIVATE KEY-----\nINVALID-OCI-INPUT-MARKER\n-----END ENCRYPTED PRIVATE KEY-----".into(), "x".repeat(64 * 1024 + 1)] {
            let error = replace_pending_oci_key(&mut pending, zeroize::Zeroizing::new(text)).unwrap_err();
            assert!(!error.contains("INVALID-OCI-INPUT-MARKER"));
            assert!(pending.as_ref().is_some_and(|(id, _)| *id == first));
        }
        let replacement = replace_pending_oci_key(
            &mut pending,
            zeroize::Zeroizing::new(format!(
                "{}\nOCI_API_KEY",
                crate::oci::tests::KEY.trim_end()
            )),
        )
        .unwrap();
        assert_ne!(replacement, first);
        assert!(pending.as_ref().is_some_and(|(id, _)| *id == replacement));
    }

    #[test]
    fn staged_oci_replacement_is_not_saved_on_cancel_or_validation_failure() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().into()).unwrap();
        let mut input: ConnectionInput =
            serde_json::from_value(serde_json::json!({"name":"OCI source test","itemType":"api"}))
                .unwrap();
        let saved = vault
            .save_oci_connection(
                input.clone(),
                None,
                crate::oci::tests::config(),
                Some(crate::oci::tests::KEY),
                None,
            )
            .unwrap();
        input.id = Some(saved.id);
        let before = std::fs::read(directory.path().join("vault.json")).unwrap();
        let mut pending = None;
        replace_pending_oci_key(
            &mut pending,
            zeroize::Zeroizing::new(crate::oci::tests::KEY.into()),
        )
        .unwrap();
        assert!(
            vault
                .save_oci_connection(
                    input,
                    Some("stale"),
                    crate::oci::tests::config(),
                    pending.as_ref().map(|(_, pem)| pem.as_str()),
                    None
                )
                .is_err()
        );
        assert!(pending.is_some());
        pending.take();
        assert!(std::fs::read(directory.path().join("vault.json")).unwrap() == before);
        assert!(
            { let item = vault.get_connection(saved.id).unwrap(); crate::oci::Credential::from_modules(&item.stored, &item.secrets).unwrap().is_configured() }
        );
        assert!(
            !serde_json::to_string(&saved)
                .unwrap()
                .contains("BEGIN PRIVATE KEY")
        );
    }
}

#[cfg(test)]
mod ssh_key_tests {
    use super::*;

    #[test]
    fn generated_key_round_trips_and_has_an_ed25519_public_key() {
        let key = PendingPrivateKey::generate().unwrap();
        let imported =
            PendingPrivateKey::parse("imported".into(), key.text.to_string(), None).unwrap();
        assert!(key.public_key.starts_with("ssh-ed25519 "));
        assert_eq!(imported.public_key, key.public_key);
        assert_eq!(imported.fingerprint, key.fingerprint);
    }

    #[test]
    fn malformed_private_keys_are_rejected() {
        assert!(
            PendingPrivateKey::parse("invalid".into(), "not a private key".into(), None).is_err()
        );
    }

    #[test]
    fn replacing_a_key_requires_a_successful_item_save() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let mut input: ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "Test SSH",
            "itemType": "ssh",
            "modules": [{"kind": "privateKey", "agentVisible": false}]
        }))
        .unwrap();
        let first = PendingPrivateKey::generate().unwrap();
        let saved = vault
            .save_connection_with_private_key(input.clone(), None, Some((&first.text, &first.name)))
            .unwrap();

        input.id = Some(saved.id);
        let replacement = PendingPrivateKey::generate().unwrap();
        assert!(
            vault
                .save_connection_with_private_key(
                    input.clone(),
                    Some("stale"),
                    Some((&replacement.text, &replacement.name))
                )
                .is_err()
        );
        let stored = vault.get_connection(saved.id).unwrap();
        assert_eq!(
            stored.secrets.private_key.as_deref(),
            Some(first.text.as_str())
        );
        assert!(!stored.stored.modules[0].agent_visible());

        vault
            .save_connection_with_private_key(
                input,
                Some(&saved.updated_at),
                Some((&replacement.text, &replacement.name)),
            )
            .unwrap();
        let stored = vault.get_connection(saved.id).unwrap();
        assert_eq!(
            stored.secrets.private_key.as_deref(),
            Some(replacement.text.as_str())
        );
    }
}
