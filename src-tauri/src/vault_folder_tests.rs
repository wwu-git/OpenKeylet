use super::*;
use serde_json::json;

fn login(name: &str, folder: &str) -> ConnectionInput {
    serde_json::from_value(json!({
        "name": name, "folder": folder, "itemType": "login", "modules": [{"kind": "password"}],
        "secrets": {"password": "synthetic-password"}
    })).unwrap()
}

fn set_grants(vault: &Vault, folders: &[&str]) -> Result<Settings> {
    vault.update_settings_patch(SettingsPatch {
        agent_write_enabled: Some(true),
        agent_write_folders: Some(folders.iter().map(|name| (*name).to_owned()).collect()),
        ..SettingsPatch::default()
    })
}

#[test]
fn folder_mutations_preserve_membership_and_revoke_grants_before_same_name_recreation() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    vault.manage_folder(FolderOperation::Create, " Blank ", None).unwrap();
    assert!(vault.manage_folder(FolderOperation::Create, "Blank", None).is_err());
    assert!(vault.manage_folder(FolderOperation::Create, "bad\u{0007}folder", None).is_err());
    assert!(set_grants(&vault, &["Not a folder"]).is_err());
    let app = |vault: &Vault, unlocked| vault.app_state("test", crate::model::BrowserBridgeState {
        enabled: false, paired: false, connected: false, status: "disabled".into(), error: String::new(), endpoint: String::new(),
    }, unlocked).unwrap();
    assert_eq!(app(&vault, true).folders, ["Blank"]);
    assert!(app(&vault, false).folders.is_empty());

    let active = vault.save_connection(login("Active", "Blank")).unwrap();
    let deleted = vault.save_connection(login("Trash", "Blank")).unwrap();
    vault.trash_connection(deleted.id, Some(&deleted.updated_at)).unwrap();
    vault.save_editor_draft(None, login("Draft", "Blank")).unwrap();
    set_grants(&vault, &["Blank"]).unwrap();
    vault.manage_folder(FolderOperation::Rename, "Blank", Some("Renamed")).unwrap();
    assert_eq!(app(&vault, true).folders, ["Renamed"]);
    assert_eq!(vault.list_editor_drafts().unwrap()[0].input.folder, "Renamed");
    assert_eq!(vault.trash_items().unwrap()[0].item.folder, "Renamed");
    let saved = vault.get_connection(active.id).unwrap().stored;
    assert_eq!(saved.folder, "Renamed");
    assert!(saved.revision > active.revision);
    assert_eq!(vault.write_capabilities().unwrap().folders, ["Renamed"]);

    let vault = Vault::open(directory.path().into()).unwrap();
    assert_eq!(app(&vault, true).folders, ["Renamed"]);
    vault.manage_folder(FolderOperation::Delete, "Renamed", None).unwrap();
    assert!(vault.get_connection(active.id).unwrap().stored.folder.is_empty());
    assert!(vault.list_editor_drafts().unwrap()[0].input.folder.is_empty());
    assert!(vault.trash_items().unwrap()[0].item.folder.is_empty());
    assert!(vault.settings().unwrap().agent_write_folders.is_empty());
    vault.manage_folder(FolderOperation::Create, "Renamed", None).unwrap();
    vault.manage_folder(FolderOperation::Create, "New scope", None).unwrap();
    let stale_settings = vault.update_settings_patch(SettingsPatch {
        agent_write_folders: Some(vec!["Renamed".into(), "New scope".into()]),
        expected_agent_write_folders: Some(vec!["Renamed".into()]),
        ..SettingsPatch::default()
    });
    assert!(stale_settings.unwrap_err().to_string().contains("permissions changed"));
    assert!(!vault.write_capabilities().unwrap().enabled);
    assert!(vault.write_capabilities().unwrap().folders.is_empty());
}

#[test]
fn historical_folder_grants_never_authorize_an_implicit_new_folder() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    let legacy = vault.save_connection(login("Old item", "Existing")).unwrap();
    let mut document = vault.read_document().unwrap();
    document.folders.clear();
    document.settings.agent_write_enabled = true;
    document.settings.agent_write_folders = vec!["Existing".into(), "Orphan".into(), "Rename destination".into()];
    vault.write_document_unlocked(&document).unwrap();
    assert_eq!(vault.write_capabilities().unwrap().folders, ["Existing"]);
    let request = serde_json::from_value(json!({
        "name": "Blocked", "itemType": "login", "folder": "Orphan", "modules": [{"kind":"password", "value":"synthetic"}]
    })).unwrap();
    assert!(vault.agent_create(request).is_err());
    vault.delete_connection(legacy.id).unwrap();
    assert_eq!(vault.read_document().unwrap().folders, ["Existing"]);

    vault.save_connection(login("Owner project", "Orphan")).unwrap();
    assert!(!vault.settings().unwrap().agent_write_folders.contains(&"Orphan".to_owned()));
    vault.manage_folder(FolderOperation::Rename, "Orphan", Some("Rename destination")).unwrap();
    assert_eq!(vault.settings().unwrap().agent_write_folders, ["Existing"]);
    vault.change_write_permissions(WritePermissionChange::AddFolder("Provisioned via CLI")).unwrap();
    assert_eq!(vault.write_capabilities().unwrap().folders, ["Existing", "Provisioned via CLI"]);
}

#[test]
fn empty_folders_survive_whole_backup_restore_without_restoring_permissions() {
    let directory = tempfile::tempdir().unwrap();
    let source = Vault::open(directory.path().join("source")).unwrap();
    source.manage_folder(FolderOperation::Create, "Empty", None).unwrap();
    set_grants(&source, &["Empty"]).unwrap();
    let cloud = crate::backup::cloud_snapshot(&source).unwrap();
    let target = Vault::open(directory.path().join("target")).unwrap();
    let mut document = target.read_document().unwrap();
    document.settings.agent_write_enabled = true;
    document.settings.agent_write_folders = vec!["Empty".into()];
    target.write_document_unlocked(&document).unwrap();
    let preview = crate::backup::preview_cloud_snapshot(&target, &cloud).unwrap();
    assert_eq!(preview.folders, ["Empty"]);
    assert!(preview.active.is_empty());
    let result = crate::backup::restore_cloud_snapshot(&target, &cloud).unwrap();
    assert_eq!(result.folders_added, 1);
    assert_eq!(result.added, 0);
    assert_eq!(target.read_document().unwrap().folders, ["Empty"]);
    assert!(target.settings().unwrap().agent_write_folders.is_empty());
    let result = crate::backup::restore_cloud_snapshot(&target, &cloud).unwrap();
    assert_eq!(result.folders_added, 0);
    let protected = directory.path().join("empty-folder.mvault");
    crate::backup::export_protected(&source, &protected, "synthetic-backup-password").unwrap();
    let local_target = Vault::open(directory.path().join("local-target")).unwrap();
    let result = crate::backup::prepare_restore(&protected, "synthetic-backup-password").unwrap()
        .restore_selected(&local_target, crate::backup::RestoreSelection { active: Vec::new(), trash: Vec::new() })
        .unwrap();
    assert_eq!(result.folders_added, 1);
    assert_eq!(local_target.read_document().unwrap().folders, ["Empty"]);
}
