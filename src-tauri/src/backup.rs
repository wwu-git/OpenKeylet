use crate::{
    crypto::{decrypt_bytes, encrypt_bytes},
    model::{ImportSummary, PortableConnection, SecretBundle, SecretEnvelope},
    vault::Vault,
};
use anyhow::{Context, Result, bail, ensure};
use atomic_write_file::AtomicWriteFile;
use base64::{Engine, engine::general_purpose::STANDARD_NO_PAD};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{Read, Write},
    path::Path,
};
use zeroize::{Zeroize, Zeroizing};

const BACKUP_VERSION: u8 = 9;
const MAX_BACKUP_BYTES: u64 = 64 * 1024 * 1024;
const PASSWORD_BACKUP_AAD: &[u8] = b"openkeylet/password-backup/v1";

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PasswordBackupFile {
    format: String,
    version: u8,
    kdf: String,
    salt: String,
    cipher: String,
    payload: SecretEnvelope,
}

fn password_backup_key(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    ensure!(salt.len() == 16, "Invalid backup salt");
    ensure!(
        !password.is_empty() && password.len() <= 4096,
        "Enter the backup password"
    );
    let params = argon2::Params::new(65_536, 3, 1, Some(32))
        .map_err(|e| anyhow::anyhow!("Invalid backup parameters: {e}"))?;
    let mut key = Zeroizing::new([0; 32]);
    argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|e| anyhow::anyhow!("Unable to derive backup key: {e}"))?;
    Ok(key)
}

pub fn export_protected(vault: &Vault, path: &Path, password: &str) -> Result<()> {
    ensure!(
        password.chars().count() >= 12 && password.len() <= 4096,
        "Use a backup password of at least 12 characters"
    );
    reject_vault_internal_export_path(vault, path)?;
    let mut salt = [0; 16];
    getrandom::fill(&mut salt).map_err(|e| anyhow::anyhow!("Random generator unavailable: {e}"))?;
    let key = password_backup_key(password, &salt)?;
    let plain = cloud_snapshot(vault)?;
    let file = PasswordBackupFile {
        format: "openkeylet-password-backup".into(),
        version: 1,
        kdf: "argon2id-64m-t3-p1".into(),
        salt: STANDARD_NO_PAD.encode(salt),
        cipher: "xchacha20poly1305".into(),
        payload: encrypt_bytes(&key, &plain, PASSWORD_BACKUP_AAD)?,
    };
    let mut writer = AtomicWriteFile::open(path).context("Unable to create backup file")?;
    writer.write_all(&serde_json::to_vec_pretty(&file)?)?;
    writer.commit()?;
    Ok(())
}

pub fn import_with_password(vault: &Vault, path: &Path, password: &str) -> Result<ImportSummary> {
    prepare_restore(path, password)?.restore_all(vault)
}

pub fn prepare_restore(path: &Path, password: &str) -> Result<PreparedBackup> {
    let file = fs::File::open(path).context("Unable to open backup")?;
    let metadata = file.metadata().context("Unable to inspect backup")?;
    ensure!(metadata.is_file(), "Choose a regular backup file");
    ensure!(metadata.len() <= MAX_BACKUP_BYTES, "Backup exceeds 64 MiB");
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_BACKUP_BYTES + 1)
        .read_to_end(&mut bytes)
        .context("Unable to read backup")?;
    ensure!(
        bytes.len() as u64 <= MAX_BACKUP_BYTES,
        "Backup exceeds 64 MiB"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("Invalid backup format")?;
    if value["format"] != "openkeylet-password-backup" {
        let file: BackupFile =
            serde_json::from_value(value).context("Invalid backup file format")?;
        validate_backup_file(&file)?;
        let plain = Zeroizing::new(decrypt_automatic_backup(&file)?);
        return parse_backup_payload(&plain, Some(file.version));
    }
    let file: PasswordBackupFile = serde_json::from_value(value)?;
    ensure!(
        file.version == 1 && file.kdf == "argon2id-64m-t3-p1" && file.cipher == "xchacha20poly1305",
        "Unsupported protected backup format"
    );
    let salt = STANDARD_NO_PAD
        .decode(&file.salt)
        .context("Invalid backup salt")?;
    let key = password_backup_key(password, &salt)?;
    let plain = Zeroizing::new(
        decrypt_bytes(&key, &file.payload, PASSWORD_BACKUP_AAD)
            .context("Incorrect backup password or damaged backup")?,
    );
    parse_backup_payload(&plain, None)
}

fn backup_aad(version: u8) -> Result<&'static [u8]> {
    match version {
        3 => Ok(b"kru/backup/v3"),
        4 => Ok(b"kru/backup/v4"),
        5 => Ok(b"kru/backup/v5"),
        6 => Ok(b"kru/backup/v6"),
        7 => Ok(b"kru/backup/v7"),
        8 => Ok(b"kru/backup/v8"),
        BACKUP_VERSION => Ok(b"kru/backup/v9"),
        _ => bail!("Unsupported backup file version"),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupFile {
    format: String,
    version: u8,
    cipher: String,
    unlock_key: String,
    payload: SecretEnvelope,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupPayload {
    format: String,
    version: u8,
    created_at: String,
    connections: Vec<BackupConnection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    folders: Vec<String>,
    #[serde(default)]
    trash: Vec<crate::model::PortableTrashItem>,
}

#[derive(Serialize, Deserialize)]
struct BackupConnection {
    connection: PortableConnection,
    secrets: SecretBundle,
}

// Only the v3 import boundary knows about obsolete connection metadata.
// Secret payloads are deserialized directly into their zeroizing type.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct V3BackupPayload {
    format: String,
    version: u8,
    connections: Vec<V3BackupConnection>,
}

#[derive(Deserialize)]
struct V3BackupConnection {
    connection: serde_json::Value,
    secrets: SecretBundle,
}

pub fn export_to_file(vault: &Vault, path: impl AsRef<Path>) -> Result<()> {
    reject_vault_internal_export_path(vault, path.as_ref())?;
    let payload = backup_payload(vault)?;
    let mut plain = serde_json::to_vec(&payload).context("Unable to serialize backup")?;
    let mut key = [0_u8; 32];
    getrandom::fill(&mut key)
        .map_err(|error| anyhow::anyhow!("Unable to generate backup key:{error}"))?;
    let encrypted = encrypt_bytes(&key, &plain, backup_aad(BACKUP_VERSION)?);
    plain.zeroize();
    let payload = encrypted?;
    let unlock_key = STANDARD_NO_PAD.encode(key);
    key.zeroize();
    let file = BackupFile {
        format: "mcp-vault-backup".to_owned(),
        version: BACKUP_VERSION,
        cipher: "xchacha20poly1305".to_owned(),
        unlock_key,
        payload,
    };
    write_backup_file(path.as_ref(), &file)
}

fn backup_payload(vault: &Vault) -> Result<BackupPayload> {
    let (connections, trash, folders) = vault.export_backup_snapshot()?;
    let connections = connections
        .into_iter()
        .map(|(connection, secrets)| BackupConnection {
            connection,
            secrets,
        })
        .collect();
    Ok(BackupPayload {
        format: "mcp-vault-portable".to_owned(),
        version: BACKUP_VERSION,
        created_at: Utc::now().to_rfc3339(),
        connections,
        trash,
        folders,
    })
}

// Cloud snapshots reuse the portable payload, never the self-unlocking .mvault
// container. The caller must encrypt these bytes with a separately held key.
pub(crate) fn cloud_snapshot(vault: &Vault) -> Result<Zeroizing<Vec<u8>>> {
    Ok(Zeroizing::new(serde_json::to_vec(&backup_payload(vault)?)?))
}

pub(crate) fn inspect_cloud_snapshot(plain: &[u8]) -> Result<(String, usize)> {
    let payload: BackupPayload = serde_json::from_slice(plain)?;
    if payload.format != "mcp-vault-portable"
        || !matches!(payload.version, 4 | 5 | 6 | 7 | 8 | BACKUP_VERSION)
    {
        bail!("Unsupported cloud backup content version");
    }
    chrono::DateTime::parse_from_rfc3339(&payload.created_at)?;
    Ok((
        payload.created_at,
        payload.connections.len() + payload.trash.len(),
    ))
}

pub(crate) fn restore_cloud_snapshot(vault: &Vault, plain: &[u8]) -> Result<ImportSummary> {
    parse_backup_payload(plain, None)?.restore_all(vault)
}

pub(crate) fn preview_cloud_snapshot(vault: &Vault, plain: &[u8]) -> Result<RestorePreview> {
    parse_backup_payload(plain, None)?.preview(vault)
}

fn write_backup_file(path: &Path, file: &BackupFile) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(&file).context("Unable to generate backup file")?;
    let mut writer = AtomicWriteFile::open(path).context("Unable to create backup file")?;
    writer
        .write_all(&bytes)
        .context("Unable to write backup file")?;
    writer.commit().context("Unable to commit backup file")?;
    Ok(())
}

fn reject_vault_internal_export_path(vault: &Vault, path: &Path) -> Result<()> {
    let data_dir =
        fs::canonicalize(vault.data_dir()).context("Unable to confirm vault directory")?;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .context("Unable to confirm current directory")?
            .join(path)
    };
    let target = if absolute.exists() {
        fs::canonicalize(&absolute).context("Unable to confirm backup destination path")?
    } else {
        let parent = absolute
            .parent()
            .context("Invalid backup destination path")?;
        let name = absolute.file_name().context("Invalid backup file name")?;
        fs::canonicalize(parent)
            .context("Unable to confirm backup destination directory")?
            .join(name)
    };
    if target.starts_with(&data_dir) {
        bail!(
            "Backup files cannot be saved inside the OpenKeylet data directory. Choose another location"
        );
    }
    Ok(())
}

pub fn import_from_file(vault: &Vault, path: impl AsRef<Path>) -> Result<ImportSummary> {
    let file = read_backup_file(path.as_ref())?;
    validate_backup_file(&file)?;
    let mut plain = decrypt_automatic_backup(&file)?;
    let result = merge_backup_payload(vault, &plain, file.version);
    plain.zeroize();
    result
}

fn read_backup_file(path: &Path) -> Result<BackupFile> {
    let bytes = fs::read(path).context("Unable to read backup file")?;
    serde_json::from_slice(&bytes).context("Invalid backup file format")
}

fn validate_backup_file(file: &BackupFile) -> Result<()> {
    if file.format != "mcp-vault-backup" {
        bail!("Unsupported backup file version");
    }
    backup_aad(file.version)?;
    if file.cipher != "xchacha20poly1305" {
        bail!("Unsupported backup encryption method");
    }
    Ok(())
}

fn decrypt_automatic_backup(file: &BackupFile) -> Result<Vec<u8>> {
    let decoded = STANDARD_NO_PAD
        .decode(&file.unlock_key)
        .context("Invalid backup auto-unlock material")?;
    let mut key: [u8; 32] = decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("Invalid backup auto-unlock material length"))?;
    let result = decrypt_bytes(&key, &file.payload, backup_aad(file.version)?);
    key.zeroize();
    result
}

fn merge_backup_payload(vault: &Vault, plain: &[u8], version: u8) -> Result<ImportSummary> {
    parse_backup_payload(plain, Some(version))?.restore_all(vault)
}

#[derive(Serialize, Deserialize)]
pub struct PreparedBackup {
    active: Vec<(PortableConnection, SecretBundle)>,
    trash: Vec<crate::model::PortableTrashItem>,
    #[serde(default)]
    folders: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreview {
    pub active: Vec<RestorePreviewRow>,
    pub trash: Vec<RestorePreviewRow>,
    pub folders: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestorePreviewRow {
    pub index: usize,
    pub name: String,
    pub item_type: String,
    pub folder: String,
    pub deleted_at: Option<String>,
    pub jump_index: Option<usize>,
    pub jump_unresolved: bool,
    pub name_conflict: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreSelection {
    pub active: Vec<usize>,
    pub trash: Vec<usize>,
}

impl PreparedBackup {
    pub fn preview(&self, vault: &Vault) -> Result<RestorePreview> {
        let active_indices = self.active_indices()?;
        let active_names = vault
            .list_connections()?
            .into_iter()
            .map(|item| name_key(&item.name))
            .collect::<HashSet<_>>();
        let trash_names = vault
            .trash_items()?
            .into_iter()
            .map(|entry| name_key(&entry.item.name))
            .collect::<HashSet<_>>();

        let row = |index, item: &PortableConnection, deleted_at, names: &HashSet<String>| {
            let jump_index = backup_jump_index(item, &active_indices);
            let item_type = if item.item_type.is_empty() {
                crate::item_types::infer(&item.modules).to_owned()
            } else {
                item.item_type.clone()
            };
            RestorePreviewRow {
                index,
                name: item.name.clone(),
                item_type,
                folder: item.folder.clone(),
                deleted_at,
                jump_index,
                jump_unresolved: item.ssh_jump_unresolved
                    || item.ssh_jump_item_id.is_some() && jump_index.is_none(),
                name_conflict: names.contains(&name_key(&item.name)),
            }
        };
        Ok(RestorePreview {
            folders: self.folders.clone(),
            active: self
                .active
                .iter()
                .enumerate()
                .map(|(index, (item, _))| row(index, item, None, &active_names))
                .collect(),
            trash: self
                .trash
                .iter()
                .enumerate()
                .map(|(index, entry)| {
                    row(
                        index,
                        &entry.connection,
                        Some(entry.deleted_at.clone()),
                        &trash_names,
                    )
                })
                .collect(),
        })
    }

    pub fn restore_selected(
        self,
        vault: &Vault,
        selection: RestoreSelection,
    ) -> Result<ImportSummary> {
        let active = checked_indices(&selection.active, self.active.len())?;
        let trash = checked_indices(&selection.trash, self.trash.len())?;
        let all_selected = active.len() == self.active.len() && trash.len() == self.trash.len();
        ensure!(
            !active.is_empty() || !trash.is_empty() || all_selected && !self.folders.is_empty(),
            "Select at least one backup item"
        );
        let active_indices = self.active_indices()?;
        let selected = self
            .active
            .iter()
            .enumerate()
            .filter(|(index, _)| active.contains(index))
            .map(|(_, (item, _))| item)
            .chain(
                self.trash
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| trash.contains(index))
                    .map(|(_, entry)| &entry.connection),
            );
        for item in selected {
            if let Some(jump) = backup_jump_index(item, &active_indices) {
                ensure!(
                    active.contains(&jump),
                    "Select the SSH jump project required by {}",
                    item.name
                );
            }
        }
        vault.merge_backup_snapshot(
            self.active
                .into_iter()
                .enumerate()
                .filter_map(|(index, entry)| active.contains(&index).then_some(entry))
                .collect(),
            self.trash
                .into_iter()
                .enumerate()
                .filter_map(|(index, entry)| trash.contains(&index).then_some(entry))
                .collect(),
            if all_selected { self.folders } else { Vec::new() },
        )
    }

    fn restore_all(self, vault: &Vault) -> Result<ImportSummary> {
        vault.merge_backup_snapshot(self.active, self.trash, self.folders)
    }

    fn active_indices(&self) -> Result<HashMap<uuid::Uuid, usize>> {
        let mut active = HashMap::new();
        for (index, (item, _)) in self.active.iter().enumerate() {
            ensure!(
                active.insert(item.id, index).is_none(),
                "Duplicate project IDs in backup"
            );
        }
        let mut trash = HashSet::new();
        for entry in &self.trash {
            let id = entry.connection.id;
            ensure!(
                !active.contains_key(&id) && trash.insert(id),
                "Duplicate project IDs in backup"
            );
        }
        Ok(active)
    }
}

fn name_key(name: &str) -> String {
    name.trim().to_lowercase()
}

fn backup_jump_index(
    item: &PortableConnection,
    active_indices: &HashMap<uuid::Uuid, usize>,
) -> Option<usize> {
    if item.ssh_jump_unresolved {
        return None;
    }
    item.ssh_jump_item_id
        .and_then(|id| active_indices.get(&id).copied())
}

fn checked_indices(indices: &[usize], count: usize) -> Result<HashSet<usize>> {
    let mut selection = HashSet::new();
    for &index in indices {
        ensure!(
            index < count && selection.insert(index),
            "Invalid backup item selection"
        );
    }
    Ok(selection)
}

fn parse_backup_payload(plain: &[u8], expected_version: Option<u8>) -> Result<PreparedBackup> {
    if expected_version == Some(3) {
        let payload: V3BackupPayload =
            serde_json::from_slice(plain).context("Backup contents are corrupted")?;
        if payload.format != "mcp-vault-portable" || payload.version != 3 {
            bail!("Unsupported backup content version");
        }
        let mut active = Vec::with_capacity(payload.connections.len());
        for mut item in payload.connections {
            if let Some(connection) = item.connection.as_object_mut() {
                for obsolete in [
                    "hostFingerprint",
                    "hostFingerprintHost",
                    "hostFingerprintPort",
                    "allowedMethods",
                    "allowedPathPrefixes",
                ] {
                    connection.remove(obsolete);
                }
            }
            let connection =
                serde_json::from_value(item.connection).context("Invalid backup item format")?;
            active.push((connection, item.secrets));
        }
        return Ok(PreparedBackup {
            active,
            trash: Vec::new(),
            folders: Vec::new(),
        });
    }
    let payload: BackupPayload =
        serde_json::from_slice(plain).context("Backup contents are corrupted")?;
    if expected_version.is_none() && payload.version == 3 {
        return parse_backup_payload(plain, Some(3));
    }
    if payload.format != "mcp-vault-portable"
        || !matches!(payload.version, 4 | 5 | 6 | 7 | 8 | BACKUP_VERSION)
        || expected_version.is_some_and(|version| version != payload.version)
    {
        bail!("Unsupported backup content version");
    }
    Ok(PreparedBackup {
        active: payload
            .connections
            .into_iter()
            .map(|item| (item.connection, item.secrets))
            .collect(),
        trash: payload.trash,
        folders: payload.folders,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn protected_backup_requires_password_and_keeps_unlock_key_out_of_file() {
        let d = tempdir().unwrap();
        let source = Vault::open(d.path().join("source")).unwrap();
        source
            .save_connection(input(
                Uuid::new_v4(),
                "Protected",
                "protected-secret-marker",
            ))
            .unwrap();
        let path = d.path().join("protected.mvault");
        assert!(export_protected(&source, &path, "short").is_err());
        export_protected(&source, &path, "synthetic-backup-password").unwrap();
        let bytes = fs::read_to_string(&path).unwrap();
        assert!(!bytes.contains("unlockKey"));
        assert!(!bytes.contains("protected-secret-marker"));
        let target = Vault::open(d.path().join("target")).unwrap();
        assert!(import_with_password(&target, &path, "wrong password").is_err());
        assert!(target.list_connections().unwrap().is_empty());
        assert_eq!(
            import_with_password(&target, &path, "synthetic-backup-password")
                .unwrap()
                .added,
            1
        );
    }
    #[test]
    fn protected_restore_previews_metadata_and_imports_only_selected_rows() {
        let dir = tempdir().unwrap();
        let source = Vault::open(dir.path().join("source")).unwrap();
        let target = Vault::open(dir.path().join("target")).unwrap();
        for name in ["Existing", "Restore this", "Skip this"] {
            source
                .save_connection(input(Uuid::new_v4(), name, "synthetic-token"))
                .unwrap();
        }
        let deleted = source
            .save_connection(input(Uuid::new_v4(), "In trash", "synthetic-token"))
            .unwrap();
        source.trash_connection(deleted.id, None).unwrap();
        let unrelated = target
            .save_connection(input(Uuid::new_v4(), "Existing", "local-token"))
            .unwrap();
        let path = dir.path().join("selection.mvault");
        let password = "synthetic-backup-password";
        export_protected(&source, &path, password).unwrap();

        assert!(prepare_restore(&path, "wrong-password").is_err());
        let prepared = prepare_restore(&path, password).unwrap();
        let preview = prepared.preview(&target).unwrap();
        assert_eq!(preview.active.len(), 3);
        assert_eq!(preview.trash.len(), 1);
        assert!(
            preview
                .active
                .iter()
                .find(|row| row.name == "Existing")
                .unwrap()
                .name_conflict
        );
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains("synthetic-token")
        );
        assert_eq!(target.list_connections().unwrap().len(), 1);
        assert!(target.trash_items().unwrap().is_empty());
        let selected = preview
            .active
            .iter()
            .find(|row| row.name == "Restore this")
            .unwrap()
            .index;

        fs::write(&path, "{}").unwrap();
        let result = prepared
            .restore_selected(
                &target,
                RestoreSelection {
                    active: vec![selected],
                    trash: vec![preview.trash[0].index],
                },
            )
            .unwrap();
        assert_eq!((result.added, result.merged), (2, 0));
        assert_eq!(
            target
                .list_connections()
                .unwrap()
                .into_iter()
                .map(|item| item.name)
                .collect::<Vec<_>>(),
            ["Existing", "Restore this"]
        );
        assert_eq!(
            target
                .get_connection(unrelated.id)
                .unwrap()
                .secrets
                .get("apiCredential"),
            Some("local-token")
        );
        assert_eq!(target.trash_items().unwrap()[0].item.name, "In trash");
    }

    #[test]
    fn selected_restore_requires_the_source_jump_and_remaps_conflicting_ids() {
        fn ssh(name: &str) -> ConnectionInput {
            serde_json::from_value(serde_json::json!({
                "name": name, "itemType": "ssh", "sshAuthType": "password",
                "modules": [
                    {"kind":"host","value":"127.0.0.1"}, {"kind":"port","value":"22"},
                    {"kind":"username"}, {"kind":"password"}
                ],
                "secrets": {"password":"synthetic-password", "namedSecrets":{"username":"fixture"}}
            }))
            .unwrap()
        }

        let dir = tempdir().unwrap();
        let source = Vault::open(dir.path().join("source")).unwrap();
        let target = Vault::open(dir.path().join("target")).unwrap();
        let jump = source.save_connection(ssh("Jump")).unwrap();
        let mut destination = ssh("Destination");
        destination.ssh_jump_item_id = Some(jump.id);
        let destination = source.save_connection(destination).unwrap();
        let mut deleted = ssh("Deleted destination");
        deleted.ssh_jump_item_id = Some(jump.id);
        let deleted = source.save_connection(deleted).unwrap();
        source.trash_connection(deleted.id, None).unwrap();
        let mut local = ssh("Jump");
        local.id = Some(jump.id);
        local.description = "existing local jump".into();
        target.save_connection(local).unwrap();

        let path = dir.path().join("legacy.mvault");
        export_to_file(&source, &path).unwrap();
        let prepared = prepare_restore(&path, "").unwrap();
        let preview = prepared.preview(&target).unwrap();
        let row = preview
            .active
            .iter()
            .find(|row| row.name == "Destination")
            .unwrap();
        let jump_index = row.jump_index.unwrap();
        assert_eq!(preview.active[jump_index].name, "Jump");
        assert_eq!(preview.trash[0].jump_index, Some(jump_index));

        for selection in [
            RestoreSelection {
                active: vec![],
                trash: vec![],
            },
            RestoreSelection {
                active: vec![usize::MAX],
                trash: vec![],
            },
            RestoreSelection {
                active: vec![jump_index, jump_index],
                trash: vec![],
            },
            RestoreSelection {
                active: vec![row.index],
                trash: vec![],
            },
            RestoreSelection {
                active: vec![],
                trash: vec![0],
            },
        ] {
            assert!(
                prepare_restore(&path, "")
                    .unwrap()
                    .restore_selected(&target, selection)
                    .is_err()
            );
        }
        assert_eq!(target.list_connections().unwrap().len(), 1);
        assert!(target.trash_items().unwrap().is_empty());
        assert_eq!(
            prepared
                .restore_selected(
                    &target,
                    RestoreSelection {
                        active: vec![jump_index, row.index],
                        trash: vec![0],
                    }
                )
                .unwrap()
                .added,
            3
        );
        let destination = target.get_connection(destination.id).unwrap();
        let restored_jump = target.ssh_jump_connection(&destination).unwrap().unwrap();
        assert_ne!(restored_jump.stored.id, jump.id);
        assert_eq!(restored_jump.stored.name, "Jump(2)");
        assert_eq!(
            target.trash_items().unwrap()[0].item.ssh_jump_item_id,
            Some(restored_jump.stored.id)
        );
    }

    #[test]
    fn older_v4_v5_and_v8_backups_import_without_a_password() {
        let d = tempdir().unwrap();
        let source = Vault::open(d.path().join("source")).unwrap();
        source
            .save_connection(input(Uuid::new_v4(), "Legacy API", "legacy-marker"))
            .unwrap();
        for version in [4, 5, 8] {
            let mut payload = serde_json::to_value(backup_payload(&source).unwrap()).unwrap();
            payload["version"] = serde_json::json!(version);
            let metadata = payload["connections"][0]["connection"]
                .as_object_mut()
                .unwrap();
            for field in ["itemType", "folder", "favorite"] {
                metadata.remove(field);
            }
            let file = BackupFile {
                format: "mcp-vault-backup".into(),
                version,
                cipher: "xchacha20poly1305".into(),
                unlock_key: STANDARD_NO_PAD.encode([42; 32]),
                payload: encrypt_bytes(
                    &[42; 32],
                    &serde_json::to_vec(&payload).unwrap(),
                    backup_aad(version).unwrap(),
                )
                .unwrap(),
            };
            let path = d.path().join(format!("v{version}.mvault"));
            write_backup_file(&path, &file).unwrap();
            let target = Vault::open(d.path().join(format!("target-{version}"))).unwrap();
            assert_eq!(import_with_password(&target, &path, "").unwrap().added, 1);
            let items = target.list_decrypted_connections().unwrap();
            assert_eq!(items[0].stored.item_type(), "api");
            assert_eq!(items[0].secrets.get("apiCredential"), Some("legacy-marker"));
        }
    }
    use super::*;
    use crate::model::{ConnectionInput, SecretBundle};
    use tempfile::tempdir;
    use uuid::Uuid;

    fn input(id: Uuid, name: &str, token: &str) -> ConnectionInput {
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
            id: Some(id),
            modules: vec![
                crate::model::ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "url".into(),
                    value: "https://api.example.test/v1/".into(),
                    ..Default::default()
                },
                crate::model::ItemModule {
                    id: uuid::Uuid::nil(),
                    kind: "apiCredential".into(),
                    ..Default::default()
                },
            ],
            name: name.into(),
            enabled: true,
            description: String::new(),
            http_auth_type: "bearer".into(),
            private_key_import_path: String::new(),
            auth_header: "X-API-Key".into(),
            auth_location: "header".into(),
            auth_prefix: String::new(),
            api_auth_headers: vec![],
            test_path: String::new(),
            remove_secret_names: vec![],
            secrets,
        }
    }

    #[test]
    fn automatic_backup_contains_no_marker_plaintext() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        vault
            .save_connection(input(
                Uuid::new_v4(),
                "private-service-marker",
                "marker-secret-123",
            ))
            .unwrap();
        let path = directory.path().join("test.mvault");
        export_to_file(&vault, &path).unwrap();
        let contents = fs::read_to_string(path).unwrap();
        assert!(!contents.contains("marker-secret-123"));
        assert!(!contents.contains("private_key"));
        assert!(contents.contains("xchacha20poly1305"));
        assert!(contents.contains("unlockKey"));
        let file: BackupFile = serde_json::from_str(&contents).unwrap();
        assert_eq!(file.version, BACKUP_VERSION);
        let plain = decrypt_automatic_backup(&file).unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(payload["version"], BACKUP_VERSION);
        assert!(
            payload["connections"][0]["connection"]
                .get("allowedMethods")
                .is_none()
        );
        assert!(
            payload["connections"][0]["connection"]
                .get("hostFingerprint")
                .is_none()
        );
    }

    #[test]
    fn v3_backup_import_preserves_all_secret_payloads_and_current_transport_settings() {
        let directory = tempdir().unwrap();
        let source = Vault::open(directory.path().join("source")).unwrap();
        let target_dir = directory.path().join("target");
        let target = Vault::open(target_dir.clone()).unwrap();
        let id = Uuid::new_v4();
        source
            .save_connection(input(id, "v3 API", "api-marker"))
            .unwrap();
        let mut fixture = serde_json::to_value(backup_payload(&source).unwrap()).unwrap();
        fixture["version"] = serde_json::json!(3);
        let metadata = fixture["connections"][0]["connection"]
            .as_object_mut()
            .unwrap();
        metadata.insert(
            "hostFingerprint".into(),
            serde_json::json!("old-fingerprint"),
        );
        metadata.insert("hostFingerprintHost".into(), serde_json::json!("old-host"));
        metadata.insert("hostFingerprintPort".into(), serde_json::json!(22));
        metadata.insert("allowedMethods".into(), serde_json::json!(["GET"]));
        metadata.insert("allowedPathPrefixes".into(), serde_json::json!(["/v1/"]));
        metadata.insert("testPath".into(), serde_json::json!("/health"));
        fixture["connections"][0]["secrets"] = serde_json::json!({
            "privateKeyName": "existing-key",
            "password": "password-marker",
            "passphrase": "passphrase-marker",
            "privateKey": "private-key-marker",
            "token": "token-marker",
            "apiKey": "api-key-marker",
            "namedSecrets": {
                "username": "username-marker",
                "apiCredential": "api-marker",
                "extra": "extra-marker"
            }
        });
        let key = [31_u8; 32];
        let plain = serde_json::to_vec(&fixture).unwrap();
        let file = BackupFile {
            format: "mcp-vault-backup".into(),
            version: 3,
            cipher: "xchacha20poly1305".into(),
            unlock_key: STANDARD_NO_PAD.encode(key),
            payload: encrypt_bytes(&key, &plain, backup_aad(3).unwrap()).unwrap(),
        };
        let path = directory.path().join("existing-v3.mvault");
        write_backup_file(&path, &file).unwrap();
        assert_eq!(import_from_file(&target, &path).unwrap().added, 1);
        drop(target);

        let reopened = Vault::open(target_dir).unwrap();
        let imported = reopened.get_connection(id).unwrap();
        assert_eq!(imported.stored.http_auth_type, "bearer");
        assert_eq!(imported.stored.auth_header, "X-API-Key");
        assert_eq!(imported.stored.base_url, "https://api.example.test/v1/");
        assert_eq!(imported.stored.test_path, "/health");
        for (field, expected) in [
            ("password", "password-marker"),
            ("passphrase", "passphrase-marker"),
            ("privateKey", "private-key-marker"),
            ("token", "token-marker"),
            ("apiKey", "api-key-marker"),
            ("username", "username-marker"),
            ("apiCredential", "api-marker"),
            ("extra", "extra-marker"),
        ] {
            assert_eq!(imported.secrets.get(field), Some(expected), "{field}");
        }
        assert_eq!(
            imported.secrets.private_key_name.as_deref(),
            Some("existing-key")
        );
        let public = imported.stored.public(Some(&imported.secrets));
        assert_eq!(public.capabilities, ["fill", "http"]);
        assert!(public.can_test);
    }

    #[test]
    fn v4_file_and_cloud_backups_remain_importable() {
        let directory = tempdir().unwrap();
        let source = Vault::open(directory.path().join("source-v4")).unwrap();
        let id = Uuid::new_v4();
        source
            .save_connection(input(id, "Existing login", "v4-secret"))
            .unwrap();
        let mut payload = backup_payload(&source).unwrap();
        payload.version = 4;
        let plain = serde_json::to_vec(&payload).unwrap();
        assert!(!String::from_utf8_lossy(&plain).contains("passkeys"));
        let key = [41_u8; 32];
        let file = BackupFile {
            format: "mcp-vault-backup".into(),
            version: 4,
            cipher: "xchacha20poly1305".into(),
            unlock_key: STANDARD_NO_PAD.encode(key),
            payload: encrypt_bytes(&key, &plain, b"kru/backup/v4").unwrap(),
        };
        let path = directory.path().join("existing-v4.mvault");
        write_backup_file(&path, &file).unwrap();
        let target = Vault::open(directory.path().join("file-target")).unwrap();
        assert_eq!(import_from_file(&target, &path).unwrap().added, 1);
        assert_eq!(
            target
                .get_connection(id)
                .unwrap()
                .secrets
                .get("apiCredential"),
            Some("v4-secret")
        );
        assert_eq!(inspect_cloud_snapshot(&plain).unwrap().1, 1);
        let cloud_target = Vault::open(directory.path().join("cloud-target")).unwrap();
        assert_eq!(
            restore_cloud_snapshot(&cloud_target, &plain).unwrap().added,
            1
        );
        assert_eq!(
            cloud_target
                .get_connection(id)
                .unwrap()
                .secrets
                .get("apiCredential"),
            Some("v4-secret")
        );
    }

    #[test]
    fn backup_version_is_bound_to_its_ciphertext() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();
        let path = directory.path().join("version.mvault");
        export_to_file(&vault, &path).unwrap();
        let mut file = read_backup_file(&path).unwrap();
        file.version = 3;
        write_backup_file(&path, &file).unwrap();
        assert!(import_from_file(&vault, &path).is_err());
    }

    #[test]
    fn export_refuses_to_overwrite_files_inside_the_vault_directory() {
        let directory = tempdir().unwrap();
        let vault = Vault::open(directory.path().join("vault")).unwrap();

        for name in ["vault.json", "master.key", "vault.lock", "backup.mvault"] {
            let error = export_to_file(&vault, vault.data_dir().join(name)).unwrap_err();
            assert!(error.to_string().contains("data directory"));
        }
    }

    #[test]
    fn import_appends_conflicts_numbers_names_and_merges_reimports() {
        let directory = tempdir().unwrap();
        let source = Vault::open(directory.path().join("source")).unwrap();
        let target = Vault::open(directory.path().join("target")).unwrap();
        let shared_id = Uuid::new_v4();
        source
            .save_connection(input(shared_id, "service", "backup-token-123"))
            .unwrap();
        source
            .save_connection(input(Uuid::new_v4(), "service(2)", "backup-token-456"))
            .unwrap();
        target
            .save_connection(input(shared_id, "service", "local-token-123"))
            .unwrap();

        let path = directory.path().join("merge.mvault");
        export_to_file(&source, &path).unwrap();
        let summary = import_from_file(&target, &path).unwrap();
        assert_eq!(summary.added, 2);
        assert_eq!(summary.merged, 0);
        let connections = target.list_connections().unwrap();
        assert_eq!(connections.len(), 3);
        assert_eq!(connections[0].name, "service");
        assert_eq!(connections[1].name, "service(2)");
        assert_eq!(connections[2].name, "service(3)");
        assert_eq!(
            target
                .get_connection(shared_id)
                .unwrap()
                .secrets
                .get("apiCredential"),
            Some("local-token-123")
        );

        let mut locally_edited = input(connections[1].id, "service(2)", "backup-token-123");
        locally_edited.description = "local note".to_owned();
        target.save_connection(locally_edited).unwrap();

        let summary = import_from_file(&target, &path).unwrap();
        assert_eq!(summary.added, 1);
        assert_eq!(summary.merged, 1);
        let entries = target.list_connections().unwrap();
        assert_eq!(entries.len(), 4);
        assert!(entries.iter().any(|item| item.id == connections[1].id && item.description == "local note"));
        assert!(entries.iter().any(|item| item.name == "service(4)" && item.description.is_empty()));
        let repeated = import_from_file(&target, &path).unwrap();
        assert_eq!(repeated.added, 0);
        assert_eq!(repeated.merged, 2);
        assert_eq!(target.list_connections().unwrap().len(), 4);
    }

    #[test]
    fn import_treats_an_existing_numbered_name_as_literal_until_the_base_exists() {
        let directory = tempdir().unwrap();
        let source = Vault::open(directory.path().join("source-numbered")).unwrap();
        let target = Vault::open(directory.path().join("target-numbered")).unwrap();
        source
            .save_connection(input(Uuid::new_v4(), "service", "shared-token"))
            .unwrap();
        source
            .save_connection(input(Uuid::new_v4(), "service(2)", "backup-token"))
            .unwrap();
        target
            .save_connection(input(Uuid::new_v4(), "service(2)", "shared-token"))
            .unwrap();

        let path = directory.path().join("numbered.mvault");
        export_to_file(&source, &path).unwrap();
        let summary = import_from_file(&target, &path).unwrap();
        assert_eq!(summary.added, 2);
        assert_eq!(summary.merged, 0);
        let names = target
            .list_connections()
            .unwrap()
            .into_iter()
            .map(|item| item.name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["service(2)", "service", "service(3)"]);

        let summary = import_from_file(&target, &path).unwrap();
        assert_eq!(summary.added, 0);
        assert_eq!(summary.merged, 2);
    }
}
