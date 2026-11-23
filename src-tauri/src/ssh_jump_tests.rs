use super::*;

fn input(name: &str) -> ConnectionInput {
    serde_json::from_value(serde_json::json!({
        "name": name, "itemType": "ssh", "sshAuthType": "password",
        "modules": [
            {"kind":"host","value":"127.0.0.1"},
            {"kind":"port","value":"22"},
            {"kind":"username"}, {"kind":"password"}
        ],
        "secrets": {"password":"simulated-password", "namedSecrets":{"username":"test-user"}}
    })).unwrap()
}

#[test]
fn jump_reference_validates_and_survives_rename_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let a = vault.save_connection(input("jump A")).unwrap();
    let mut b_input = input("target B");
    b_input.ssh_jump_item_id = Some(a.id);
    let b = vault.save_connection(b_input.clone()).unwrap();
    assert_eq!(b.ssh_jump_item_id, Some(a.id));
    let mut rename = input("renamed A");
    rename.id = Some(a.id);
    vault.save_connection(rename).unwrap();
    assert_eq!(vault.ssh_jump_connection(&vault.get_connection(b.id).unwrap()).unwrap().unwrap().stored.name, "renamed A");
    let entries = vault.export_connections().unwrap();
    let restored_dir = tempfile::tempdir().unwrap();
    let restored = Vault::open(restored_dir.path().into()).unwrap();
    assert_eq!(restored.merge_connections(entries.clone()).unwrap().added, 2);
    assert_eq!(restored.merge_connections(entries).unwrap().added, 0);
    vault.delete_connection(a.id).unwrap();
    assert!(vault.ssh_jump_connection(&vault.get_connection(b.id).unwrap()).is_err());
}

#[test]
fn self_and_multiple_hops_are_rejected_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let a = vault.save_connection(input("A")).unwrap();
    let mut b = input("B"); b.ssh_jump_item_id = Some(a.id);
    let b = vault.save_connection(b).unwrap();
    let mut a_edit = input("A"); a_edit.id = Some(a.id); a_edit.ssh_jump_item_id = Some(b.id);
    assert!(vault.save_connection(a_edit.clone()).is_err());
    a_edit.ssh_jump_item_id = Some(a.id);
    assert!(vault.save_connection(a_edit).is_err());
    assert!(vault.get_connection(a.id).unwrap().stored.ssh_jump_item_id.is_none());
}

#[test]
fn restore_remaps_conflicting_jump_and_stays_idempotent() {
    let source_dir = tempfile::tempdir().unwrap();
    let source = Vault::open(source_dir.path().into()).unwrap();
    let a = source.save_connection(input("A")).unwrap();
    let mut b = input("B"); b.ssh_jump_item_id = Some(a.id);
    let b = source.save_connection(b).unwrap();
    let dest_dir = tempfile::tempdir().unwrap();
    let dest = Vault::open(dest_dir.path().into()).unwrap();
    let mut local_a = input("A"); local_a.id = Some(a.id); local_a.description = "local conflict".into();
    dest.save_connection(local_a).unwrap();
    let entries = source.export_connections().unwrap();
    assert_eq!(dest.merge_connections(entries.clone()).unwrap().added, 2);
    let restored_b = dest.get_connection(b.id).unwrap();
    let mapped_a = restored_b.stored.ssh_jump_item_id.unwrap();
    assert_ne!(mapped_a, a.id);
    assert_eq!(dest.get_connection(mapped_a).unwrap().stored.name, "A(2)");
    assert_eq!(dest.merge_connections(entries).unwrap().added, 0);
    assert_eq!(dest.get_connection(b.id).unwrap().stored.ssh_jump_item_id, Some(mapped_a));
}

#[test]
fn missing_imported_jump_cannot_bind_to_same_local_id() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let a = vault.save_connection(input("A")).unwrap();
    let mut b = input("B"); b.ssh_jump_item_id = Some(a.id);
    let b = vault.save_connection(b).unwrap();
    let entries = vault.export_connections().unwrap().into_iter().filter(|(entry,_)|entry.id==b.id).collect();
    assert_eq!(vault.merge_connections(entries).unwrap().added, 1);
    let imported = vault.list_connections().unwrap().into_iter().find(|entry|entry.name=="B(2)").unwrap();
    assert!(imported.ssh_jump_unresolved);
    assert!(vault.ssh_jump_connection(&vault.get_connection(imported.id).unwrap()).is_err());
}

#[test]
fn trust_is_local_and_invalid_fingerprint_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    assert!(vault.set_ssh_host_trust("localhost",22,Some("SHA256:bad")).is_err());
    let fingerprint = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    vault.set_ssh_host_trust("LOCALHOST",22,Some(fingerprint)).unwrap();
    assert_eq!(vault.ssh_host_trust().unwrap()[0].host,"localhost");
    assert!(vault.export_connections().unwrap().is_empty());
    vault.set_ssh_host_trust("localhost",22,None).unwrap();
    assert!(vault.ssh_host_trust().unwrap().is_empty());
}

#[test]
fn duplicate_snapshot_ids_reject_without_partial_writes() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    vault.save_connection(input("A")).unwrap();
    let entries = vault.export_connections().unwrap();
    let dest_dir = tempfile::tempdir().unwrap();
    let dest = Vault::open(dest_dir.path().into()).unwrap();
    assert!(dest.merge_connections(vec![entries[0].clone(), entries[0].clone()]).is_err());
    assert!(dest.list_connections().unwrap().is_empty());
    let trash = crate::model::PortableTrashItem { deleted_at: Utc::now().to_rfc3339(), connection: entries[0].0.clone(), secrets: entries[0].1.clone() };
    assert!(dest.merge_backup_entries(entries, vec![trash]).is_err());
    assert!(dest.list_connections().unwrap().is_empty());
}
