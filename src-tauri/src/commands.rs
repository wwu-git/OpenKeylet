use crate::{
    agent_registry::{AgentActionResult, AgentClientStatus, AgentRegistry},
    backup,
    browser::BrowserBridge,
    executor,
    github_backup::{self, BackupStatus, ConnectInput, GithubBackup, RestoreResult},
    gui_instance::GuiInstance,
    mcp,
    model::{
        AppState, ConnectionInput, ImportSummary, NewActivity, OwnerEditorDraft, OwnerLockState,
        OwnerSecretView, PublicConnection, SettingsPatch,
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
    pending_private_key: Mutex<Option<PendingPrivateKey>>,
    pending_oci_key: Mutex<Option<(Uuid, zeroize::Zeroizing<String>)>>,
    pending_import: Mutex<Option<PendingImport>>,
    owner_session: Mutex<OwnerSession>,
    github_backup: Mutex<GithubBackup>,
    agent_registry: AgentRegistry,
    tray_available: AtomicBool,
}

struct PendingImport {
    id: String,
    created: Instant,
    encrypted: crate::model::SecretEnvelope,
}

#[derive(Default)]
struct OwnerSession {
    unlocked: bool,
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
    let pin_configured = runtime
        .vault
        .owner_pin_configured()
        .map_err(command_error)?;
    let mut session = runtime.owner_session.lock().await;
    if !pin_configured {
        session.unlocked = false;
        return Ok(OwnerLockState {
            pin_configured,
            unlocked: true,
            expires_in_seconds: 0,
        });
    }
    Ok(OwnerLockState {
        pin_configured,
        unlocked: session.unlocked,
        expires_in_seconds: 0,
    })
}

async fn require_owner_unlocked(runtime: &AppRuntime) -> Result<(), String> {
    if owner_lock_state(runtime).await?.unlocked {
        Ok(())
    } else {
        Err("GUI is locked; enter the six-digit PIN".to_owned())
    }
}

async fn keep_owner_unlocked(runtime: &AppRuntime) {
    runtime.owner_session.lock().await.unlocked = true;
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
    runtime
        .vault
        .app_state(&runtime.executable, browser)
        .map_err(command_error)
}

#[tauri::command]
async fn owner_status(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_set_pin(
    runtime: State<'_, AppRuntime>,
    pin: String,
) -> Result<OwnerLockState, String> {
    runtime.vault.set_owner_pin(&pin).map_err(command_error)?;
    keep_owner_unlocked(&runtime).await;
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_disable_pin(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    require_owner_unlocked(&runtime).await?;
    runtime.vault.disable_owner_pin().map_err(command_error)?;
    runtime.owner_session.lock().await.unlocked = false;
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_unlock(
    runtime: State<'_, AppRuntime>,
    pin: String,
) -> Result<OwnerLockState, String> {
    if !runtime
        .vault
        .verify_owner_pin(&pin)
        .map_err(command_error)?
    {
        return Err("Incorrect PIN".to_owned());
    }
    keep_owner_unlocked(&runtime).await;
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_lock(runtime: State<'_, AppRuntime>) -> Result<OwnerLockState, String> {
    runtime.owner_session.lock().await.unlocked = false;
    *runtime.pending_import.lock().await = None;
    runtime.pending_private_key.lock().await.take();
    runtime.pending_oci_key.lock().await.take();
    owner_lock_state(&runtime).await
}

#[tauri::command]
async fn owner_secret_view(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
) -> Result<OwnerSecretView, String> {
    require_owner_unlocked(&runtime).await?;
    keep_owner_unlocked(&runtime).await;
    runtime.vault.owner_secret_view(id).map_err(command_error)
}

#[tauri::command]
async fn owner_editor_drafts(
    runtime: State<'_, AppRuntime>,
) -> Result<Vec<OwnerEditorDraft>, String> {
    require_owner_unlocked(&runtime).await?;
    keep_owner_unlocked(&runtime).await;
    runtime.vault.list_editor_drafts().map_err(command_error)
}

#[tauri::command]
async fn save_editor_draft(
    runtime: State<'_, AppRuntime>,
    draft_id: Option<Uuid>,
    mut input: ConnectionInput,
) -> Result<OwnerEditorDraft, String> {
    require_owner_unlocked(&runtime).await?;
    keep_owner_unlocked(&runtime).await;
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
) -> Result<(), String> {
    require_owner_unlocked(&runtime).await?;
    keep_owner_unlocked(&runtime).await;
    app.clipboard().write_text(&value).map_err(command_error)
}

#[tauri::command]
fn oci_regions() -> &'static [&'static str] {
    crate::oci::REGIONS
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct OciEditorState {
    config: crate::oci::Config,
    has_key: bool,
}

#[tauri::command]
async fn oci_editor_state(
    runtime: State<'_, AppRuntime>,
    id: Uuid,
) -> Result<OciEditorState, String> {
    require_owner_unlocked(&runtime).await?;
    let item = runtime.vault.get_connection(id).map_err(command_error)?;
    let credential = item
        .secrets
        .oci
        .as_ref()
        .ok_or_else(|| "Missing OCI configuration".to_owned())?;
    Ok(OciEditorState {
        config: credential.config.clone(),
        has_key: credential.is_configured(),
    })
}

#[tauri::command]
async fn stage_oci_key(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
) -> Result<Option<Uuid>, String> {
    use std::io::Read;
    require_owner_unlocked(&runtime).await?;
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
    crate::oci::validate_pem(&pem).map_err(command_error)?;
    let mut pending = runtime.pending_oci_key.lock().await;
    require_owner_unlocked(&runtime).await?;
    let id = Uuid::new_v4();
    *pending = Some((id, pem));
    Ok(Some(id))
}

#[tauri::command]
async fn discard_oci_key(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    let mut pending = runtime.pending_oci_key.lock().await;
    if pending.as_ref().is_some_and(|(key_id, _)| *key_id == id) {
        pending.take();
    }
    Ok(())
}

#[tauri::command]
async fn save_oci_connection(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    mut input: ConnectionInput,
    expected_updated_at: Option<String>,
    config: crate::oci::Config,
    pending_id: Option<Uuid>,
) -> Result<PublicConnection, String> {
    let mut pending = runtime.pending_oci_key.lock().await;
    require_owner_unlocked(&runtime).await?;
    let key = pending_id
        .map(|requested| {
            pending
                .as_ref()
                .filter(|(id, _)| *id == requested)
                .map(|(_, pem)| pem.as_str())
                .ok_or_else(|| "Please select the OCI signing key again".to_owned())
        })
        .transpose()?;
    let mut pending_ssh = runtime.pending_private_key.lock().await;
    let ssh_key = staged_ssh_key(&mut input, pending_ssh.as_ref())?;
    let saved = runtime
        .vault
        .save_oci_connection(input, expected_updated_at.as_deref(), config, key, ssh_key)
        .map_err(command_error)?;
    if ssh_key.is_some() {
        pending_ssh.take();
    }
    drop(pending_ssh);
    if pending_id.is_some() {
        pending.take();
    }
    drop(pending);
    emit_changed(&app);
    Ok(saved)
}

#[tauri::command]
async fn save_connection(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    mut input: ConnectionInput,
    expected_updated_at: Option<String>,
) -> Result<PublicConnection, String> {
    require_owner_unlocked(&runtime).await?;
    let mut pending = runtime.pending_private_key.lock().await;
    let private_key = staged_ssh_key(&mut input, pending.as_ref())?;
    let connection = runtime
        .vault
        .save_connection_with_private_key(input, expected_updated_at.as_deref(), private_key)
        .map_err(command_error)?;
    if private_key.is_some() {
        pending.take();
    }
    drop(pending);
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
    let connection = runtime.vault.get_connection(id).map_err(command_error)?;
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
    _runtime: State<'_, AppRuntime>,
    format: String,
) -> Result<String, String> {
    let config = mcp::render_config(&format).map_err(command_error)?;
    app.clipboard().write_text(&config).map_err(command_error)?;
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
    russh::keys::decode_secret_key(text.trim(), passphrase.filter(|value| !value.is_empty()))
        .map_err(|_| anyhow::anyhow!("Invalid SSH private key or incorrect key passphrase"))
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
    require_owner_unlocked(&runtime).await?;
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
    require_owner_unlocked(&runtime).await?;
    let preview = PrivateKeyPreview {
        id: pending.id,
        name: pending.name.clone(),
        fingerprint: pending.fingerprint.clone(),
    };
    *runtime.pending_private_key.lock().await = Some(pending);
    Ok(Some(preview))
}

#[tauri::command]
async fn discard_private_key(runtime: State<'_, AppRuntime>, id: Uuid) -> Result<(), String> {
    let mut pending = runtime.pending_private_key.lock().await;
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
    require_owner_unlocked(&runtime).await?;
    let public_key = if let Some(id) = pending_id {
        runtime
            .pending_private_key
            .lock()
            .await
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
    app.clipboard()
        .write_text(public_key)
        .map_err(command_error)
}

#[tauri::command]
async fn quick_pair_browser(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    port: u16,
) -> Result<bool, String> {
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

    let browser_opened = open_chromium_extensions_page();
    emit_changed(&app);

    Ok(browser_opened)
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

#[cfg(windows)]
fn open_chromium_extensions_page() -> bool {
    let mut candidates = Vec::new();
    for root in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        let Some(root) = std::env::var_os(root) else {
            continue;
        };
        let root = PathBuf::from(root);
        candidates.push(root.join("Google/Chrome/Application/chrome.exe"));
        candidates.push(root.join("Microsoft/Edge/Application/msedge.exe"));
        candidates.push(root.join("BraveSoftware/Brave-Browser/Application/brave.exe"));
    }
    candidates.extend(["chrome.exe", "msedge.exe", "brave.exe"].map(PathBuf::from));
    candidates.into_iter().any(|candidate| {
        if candidate.components().count() > 1 && !candidate.is_file() {
            return false;
        }
        Command::new(candidate)
            .arg("--new-tab")
            .arg("chrome://extensions")
            .spawn()
            .is_ok()
    })
}

#[cfg(target_os = "macos")]
fn open_chromium_extensions_page() -> bool {
    ["Google Chrome", "Microsoft Edge", "Brave Browser"]
        .into_iter()
        .any(|browser| {
            Command::new("open")
                .args(["-a", browser, "chrome://extensions"])
                .spawn()
                .is_ok()
        })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_chromium_extensions_page() -> bool {
    [
        "google-chrome",
        "microsoft-edge",
        "brave-browser",
        "chromium",
    ]
    .into_iter()
    .any(|browser| {
        Command::new(browser)
            .args(["--new-tab", "chrome://extensions"])
            .spawn()
            .is_ok()
    })
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
    require_owner_unlocked(&runtime).await?;
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
    let path = path.into_path().map_err(command_error)?;
    backup::export_protected(&runtime.vault, &path, &password).map_err(command_error)?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

#[tauri::command]
async fn github_backup_status(runtime: State<'_, AppRuntime>) -> Result<BackupStatus, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .status()
        .map_err(command_error)
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
async fn github_backup_generate_key(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    require_owner_unlocked(&runtime).await?;
    github_backup::generate_recovery_key().map_err(command_error)
}

#[tauri::command]
async fn github_backup_recovery_key(runtime: State<'_, AppRuntime>) -> Result<String, String> {
    require_owner_unlocked(&runtime).await?;
    runtime
        .github_backup
        .lock()
        .await
        .recovery_key()
        .map_err(command_error)
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
async fn github_backup_restore(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
) -> Result<RestoreResult, String> {
    require_owner_unlocked(&runtime).await?;
    let result = runtime
        .github_backup
        .lock()
        .await
        .restore()
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
async fn import_backup(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    password: String,
) -> Result<Option<ImportSummary>, String> {
    require_owner_unlocked(&runtime).await?;
    let password = zeroize::Zeroizing::new(password);
    let selected = app
        .dialog()
        .file()
        .add_filter("OpenKeylet backup", &["mvault"])
        .blocking_pick_file();
    let Some(path) = selected else {
        return Ok(None);
    };
    let path = path.into_path().map_err(command_error)?;
    let summary =
        backup::import_with_password(&runtime.vault, &path, &password).map_err(command_error)?;
    emit_changed(&app);
    Ok(Some(summary))
}

#[tauri::command]
async fn prepare_import(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    format: String,
) -> Result<Option<serde_json::Value>, String> {
    require_owner_unlocked(&runtime).await?;
    *runtime.pending_import.lock().await = None;
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
    require_owner_unlocked(&runtime).await?;
    let parsed = crate::migration::read(&path.into_path().map_err(command_error)?, &format)
        .map_err(command_error)?;
    let id = Uuid::new_v4().to_string();
    let encrypted = runtime
        .vault
        .protect_local_state(&parsed.entries)
        .map_err(command_error)?;
    let result = serde_json::json!({"id":id,"preview":parsed.preview});
    *runtime.pending_import.lock().await = Some(PendingImport {
        id,
        created: Instant::now(),
        encrypted,
    });
    Ok(Some(result))
}

#[tauri::command]
async fn commit_import(
    app: AppHandle,
    runtime: State<'_, AppRuntime>,
    id: String,
) -> Result<ImportSummary, String> {
    require_owner_unlocked(&runtime).await?;
    let mut slot = runtime.pending_import.lock().await;
    let pending = slot.as_ref().ok_or("Choose an import file first")?;
    if pending.id != id {
        return Err("Import preview changed; choose the file again".into());
    }
    if pending.created.elapsed().as_secs() > 300 {
        *slot = None;
        return Err("Import preview expired; choose the file again".into());
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
    *runtime.pending_import.lock().await = None;
    Ok(())
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
    let icon = app
        .default_window_icon()
        .context("OpenKeylet tray icon not found")?
        .clone();

    TrayIconBuilder::with_id("openkeylet-tray")
        .icon(icon)
        .tooltip("OpenKeylet — LOCAL VAULT")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id() == "tray-open" {
                show_main_window(app);
            } else if event.id() == "tray-lock" {
                let handle = app.clone();
                tauri::async_runtime::spawn(async move {
                    let pin_configured = handle.state::<AppRuntime>().vault.owner_pin_configured();
                    handle
                        .state::<AppRuntime>()
                        .owner_session
                        .lock()
                        .await
                        .unlocked = false;
                    show_main_window(&handle);
                    if matches!(pin_configured, Ok(false)) {
                        let _ = handle.emit("pin-setup-requested", ());
                    }
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
    let data_dir = app_data_dir()?;
    let mut gui_instance = GuiInstance::acquire(&data_dir)?;
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
                pending_private_key: Mutex::new(None),
                pending_oci_key: Mutex::new(None),
                pending_import: Mutex::new(None),
                owner_session: Mutex::new(OwnerSession::default()),
                agent_registry,
                tray_available: AtomicBool::new(false),
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
            owner_set_pin,
            owner_disable_pin,
            owner_unlock,
            owner_lock,
            owner_secret_view,
            owner_editor_drafts,
            save_editor_draft,
            delete_editor_draft,
            copy_owner_value,
            save_connection,
            oci_regions,
            oci_editor_state,
            stage_oci_key,
            discard_oci_key,
            save_oci_connection,
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
            reset_browser_pairing,
            open_browser_extension_folder,
            drag_browser_extension,
            export_backup,
            generate_password,
            import_backup,
            prepare_import,
            commit_import,
            cancel_import,
            github_backup_status,
            github_backup_account,
            github_backup_generate_key,
            github_backup_recovery_key,
            github_backup_connect,
            github_backup_upload,
            github_backup_restore,
            github_backup_disconnect,
            github_backup_help,
            open_data_folder,
            window_action,
        ])
        .build(tauri::generate_context!())
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
