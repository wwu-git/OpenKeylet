use super::*;
use crate::model::OciModuleBindings;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Vault, PublicConnection) {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().join("vault")).unwrap();
    let saved = vault
        .save_oci_connection(
            serde_json::from_value(json!({"name":"OCI modules","folder":"Agent"})).unwrap(),
            None,
            crate::oci::tests::config(),
            Some(crate::oci::tests::KEY),
            None,
        )
        .unwrap();
    (directory, vault, saved)
}

fn edit(vault: &Vault, id: Uuid) -> ConnectionInput {
    let item = vault.get_connection(id).unwrap();
    serde_json::from_value(serde_json::to_value(item.stored.portable()).unwrap()).unwrap()
}

#[test]
fn oci_modules_rename_reorder_delete_and_incomplete_save_preserve_bound_values() {
    let (_directory, vault, saved) = fixture();
    let mut input = edit(&vault, saved.id);
    let bindings = input.oci_bindings.clone().unwrap();
    let region = input
        .modules
        .iter_mut()
        .find(|module| Some(module.id) == bindings.region)
        .unwrap();
    region.name = "renamed_region".into();
    region.agent_visible = Some(false);
    input.modules.reverse();
    vault.save_connection(input).unwrap();
    let item = vault.get_connection(saved.id).unwrap();
    assert_eq!(
        crate::oci::Credential::from_modules(&item.stored, &item.secrets)
            .unwrap()
            .config
            .region,
        "ap-tokyo-1"
    );
    assert_eq!(item.secrets.get("region"), None);
    assert_eq!(item.secrets.get("renamed_region"), Some("ap-tokyo-1"));
    let mut input = edit(&vault, saved.id);
    input
        .modules
        .retain(|module| Some(module.id) != bindings.private_key);
    vault.save_connection(input).unwrap();
    let item = vault.get_connection(saved.id).unwrap();
    assert!(
        item.stored
            .oci_bindings
            .as_ref()
            .unwrap()
            .private_key
            .is_none()
    );
    assert!(item.secrets.get("privateKey").is_none());
    assert!(
        crate::oci::Credential::from_modules(&item.stored, &item.secrets)
            .unwrap_err()
            .to_string()
            .contains("private key")
    );
    let mut input = edit(&vault, saved.id);
    input.modules.clear();
    vault.save_connection(input).unwrap();
    let item = vault.get_connection(saved.id).unwrap();
    assert!(item.stored.modules.is_empty());
    assert!(item.secrets.non_empty_values().is_empty());
}

#[test]
fn oci_bindings_reject_cross_project_ids_and_invalid_replacements_atomically() {
    let (_directory, vault, saved) = fixture();
    let before = fs::read(vault.data_dir().join("vault.json")).unwrap();
    let mut input = edit(&vault, saved.id);
    input.oci_bindings.as_mut().unwrap().region = Some(Uuid::new_v4());
    assert!(vault.save_connection(input).is_err());
    assert_eq!(
        fs::read(vault.data_dir().join("vault.json")).unwrap(),
        before
    );
    let mut input = edit(&vault, saved.id);
    input.secrets.private_key = Some("invalid RSA replacement".into());
    assert!(vault.save_connection(input).is_err());
    assert_eq!(
        fs::read(vault.data_dir().join("vault.json")).unwrap(),
        before
    );
    let item = vault.get_connection(saved.id).unwrap();
    assert_eq!(item.secrets.get("privateKey"), Some(crate::oci::tests::KEY));
}

#[test]
fn oci_backup_conflicts_and_repeated_restores_keep_module_bindings_and_key() {
    let (directory, vault, saved) = fixture();
    let target = Vault::open(directory.path().join("target")).unwrap();
    let first = crate::backup::cloud_snapshot(&vault).unwrap();
    assert_eq!(
        crate::backup::restore_cloud_snapshot(&target, &first)
            .unwrap()
            .added,
        1
    );
    let mut input = edit(&vault, saved.id);
    input.description = "changed public purpose".into();
    vault.save_connection(input).unwrap();
    let changed = crate::backup::cloud_snapshot(&vault).unwrap();
    assert_eq!(
        crate::backup::restore_cloud_snapshot(&target, &changed)
            .unwrap()
            .added,
        1
    );
    for _ in 0..2 {
        let summary = crate::backup::restore_cloud_snapshot(&target, &changed).unwrap();
        assert_eq!((summary.added, summary.merged), (0, 1));
        assert_eq!(target.list_connections().unwrap().len(), 2);
    }
    for public in target.list_connections().unwrap() {
        let item = target.get_connection(public.id).unwrap();
        assert!(crate::oci::Credential::from_modules(&item.stored, &item.secrets).is_ok());
    }
    let mut exported = vault.export_connections().unwrap();
    let (public, _) = &mut exported[0];
    let old_ids = public
        .modules
        .iter()
        .map(|module| module.id)
        .collect::<Vec<_>>();
    let mut mapping = std::collections::HashMap::new();
    for module in &mut public.modules {
        let next = Uuid::new_v4();
        mapping.insert(module.id, next);
        module.id = next;
    }
    let bindings = public.oci_bindings.as_mut().unwrap();
    for id in [
        &mut bindings.region,
        &mut bindings.service_url,
        &mut bindings.tenancy_ocid,
        &mut bindings.user_ocid,
        &mut bindings.fingerprint,
        &mut bindings.private_key,
    ] {
        *id = id.map(|id| mapping[&id]);
    }
    assert!(!old_ids.contains(&public.modules[0].id));
    assert_eq!(target.merge_connections(exported).unwrap().added, 0);
}

#[test]
fn legacy_oci_migration_is_backed_up_idempotent_and_does_not_overwrite_fields() {
    let (directory, vault, saved) = fixture();
    let mut document = vault.read_document().unwrap();
    let stored = &mut document.connections[0];
    let mut legacy_secret = SecretBundle::default();
    legacy_secret.oci = Some(
        crate::oci::Credential::new(crate::oci::tests::config(), crate::oci::tests::KEY).unwrap(),
    );
    legacy_secret.named_secrets = serde_json::from_value(
        json!({"Region":"independent custom value", "tenancy-ocid":"another custom value"}),
    )
    .unwrap();
    stored.modules.retain(|module| module.kind == "url");
    stored.modules.push(ItemModule {
        id: Uuid::new_v4(),
        kind: "customSecret".into(),
        name: "Region".into(),
        value: String::new(),
        agent_visible: Some(false),
        alternate_values: vec![],
    });
    stored.modules.push(ItemModule {
        id: Uuid::new_v4(),
        kind: "customSecret".into(),
        name: "tenancy-ocid".into(),
        value: String::new(),
        agent_visible: Some(false),
        alternate_values: vec![],
    });
    stored.oci_bindings = None;
    stored.encrypted_secrets = vault.inner.key.encrypt(&legacy_secret).unwrap();
    document.version = 15;
    vault.write_document_unlocked(&document).unwrap();
    let before = fs::read(vault.data_dir().join("vault.json")).unwrap();
    drop(vault);
    let migrated = Vault::open(directory.path().join("vault")).unwrap();
    let item = migrated.get_connection(saved.id).unwrap();
    assert!(item.secrets.oci.is_none());
    assert_eq!(item.secrets.get("Region"), Some("independent custom value"));
    assert_eq!(
        item.secrets.get("tenancy-ocid"),
        Some("another custom value")
    );
    assert_eq!(
        crate::oci::Credential::from_modules(&item.stored, &item.secrets)
            .unwrap()
            .config,
        crate::oci::tests::config()
    );
    let bindings = item.stored.oci_bindings.as_ref().unwrap();
    for (id, visible) in [
        (bindings.region, true),
        (bindings.tenancy_ocid, true),
        (bindings.service_url, true),
        (bindings.private_key, false),
        (bindings.user_ocid, false),
        (bindings.fingerprint, false),
    ] {
        assert_eq!(
            item.stored
                .modules
                .iter()
                .find(|module| Some(module.id) == id)
                .unwrap()
                .agent_visible(),
            visible
        );
    }
    let backups = fs::read_dir(migrated.data_dir())
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("before-oci-modules-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(
        fs::read(backups[0].path().join("vault.json")).unwrap(),
        before
    );
    let after = fs::read(migrated.data_dir().join("vault.json")).unwrap();
    drop(migrated);
    let reopened = Vault::open(directory.path().join("vault")).unwrap();
    assert_eq!(
        fs::read(reopened.data_dir().join("vault.json")).unwrap(),
        after
    );
    assert_eq!(reopened.list_connections().unwrap().len(), 1);
}

#[test]
fn migration_backup_failure_leaves_original_vault_untouched() {
    let (_directory, vault, _) = fixture();
    let mut document = vault.read_document().unwrap();
    document.version = 15;
    vault.write_document_unlocked(&document).unwrap();
    let before = fs::read(vault.data_dir().join("vault.json")).unwrap();
    fs::remove_file(vault.data_dir().join("master.key")).unwrap();
    assert!(vault.migrate_oci_modules().is_err());
    assert_eq!(
        fs::read(vault.data_dir().join("vault.json")).unwrap(),
        before
    );
}

#[test]
fn agent_can_create_bind_and_edit_oci_modules_without_changing_visibility() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    vault
        .update_settings_patch(
            serde_json::from_value(json!({"agentWriteEnabled":true,"agentWriteFolders":["Agent"]}))
                .unwrap(),
        )
        .unwrap();
    let created = vault.agent_create(serde_json::from_value(json!({"name":"Agent OCI","folder":"Agent","itemType":"api","httpAuthType":"oci-signature","modules":[{"kind":"customSecret","name":"region","value":"ap-tokyo-1"},{"kind":"url","value":"https://iaas.ap-tokyo-1.oraclecloud.com/20160918/"},{"kind":"customSecret","name":"tenancy_ocid","value":"ocid1.tenancy.oc1..test"},{"kind":"customSecret","name":"user_ocid","value":"ocid1.user.oc1..test"},{"kind":"customSecret","name":"fingerprint","value":crate::oci::tests::config().fingerprint},{"kind":"privateKey","value":crate::oci::tests::KEY}]})).unwrap()).unwrap();
    let id = created.id.parse().unwrap();
    let modules = vault.get_connection(id).unwrap().stored.modules;
    let bindings = OciModuleBindings {
        region: Some(modules[0].id),
        service_url: Some(modules[1].id),
        tenancy_ocid: Some(modules[2].id),
        user_ocid: Some(modules[3].id),
        fingerprint: Some(modules[4].id),
        private_key: Some(modules[5].id),
    };
    let bound = vault
        .agent_update(
            serde_json::from_value(
                json!({"id":created.id,"expectedRevision":created.revision,"ociBindings":bindings}),
            )
            .unwrap(),
        )
        .unwrap();
    let item = vault.get_connection(id).unwrap();
    assert!(crate::oci::Credential::from_modules(&item.stored, &item.secrets).is_ok());
    assert!(!item.stored.modules[5].agent_visible());
    assert!(vault.agent_update(serde_json::from_value(json!({"id":created.id,"expectedRevision":created.revision,"description":"stale"})).unwrap()).is_err());
    let next = vault.agent_update(serde_json::from_value(json!({"id":created.id,"expectedRevision":bound.revision,"ociBindings":{"privateKey":null}})).unwrap()).unwrap();
    let item = vault.get_connection(id).unwrap();
    assert!(
        item.stored
            .oci_bindings
            .as_ref()
            .unwrap()
            .private_key
            .is_none()
    );
    assert_eq!(
        item.stored.oci_bindings.as_ref().unwrap().region,
        bindings.region
    );
    assert!(crate::oci::Credential::from_modules(&item.stored, &item.secrets).is_err());
    vault
        .update_settings_patch(serde_json::from_value(json!({"agentWriteEnabled":false})).unwrap())
        .unwrap();
    assert!(vault.agent_update(serde_json::from_value(json!({"id":created.id,"expectedRevision":next.revision,"description":"revoked"})).unwrap()).is_err());
}

#[test]
fn legacy_active_and_trash_snapshots_restore_idempotently_with_or_without_key() {
    for has_key in [true, false] {
        let (directory, source, _) = fixture();
        let (mut public, mut secrets) = source.export_connections().unwrap().remove(0);
        public.oci_bindings = None;
        public.modules.retain(|module| module.kind == "url");
        secrets.private_key = None;
        secrets.named_secrets = Default::default();
        let mut credential =
            crate::oci::Credential::new(crate::oci::tests::config(), crate::oci::tests::KEY)
                .unwrap();
        if !has_key {
            credential.key_pem = None;
        }
        secrets.oci = Some(credential);
        let mut trash = public.clone();
        trash.id = Uuid::new_v4();
        trash.name = "Legacy trash OCI".into();
        let target = Vault::open(directory.path().join("legacy-target")).unwrap();
        let entries = || {
            vec![crate::model::PortableTrashItem {
                deleted_at: Utc::now().to_rfc3339(),
                connection: trash.clone(),
                secrets: secrets.clone(),
            }]
        };
        let first = target
            .merge_backup_entries(vec![(public.clone(), secrets.clone())], entries())
            .unwrap();
        assert_eq!((first.added, first.merged), (2, 0));
        for _ in 0..2 {
            let next = target
                .merge_backup_entries(vec![(public.clone(), secrets.clone())], entries())
                .unwrap();
            assert_eq!((next.added, next.merged), (0, 2));
            let document = target.read_document().unwrap();
            assert_eq!((document.connections.len(), document.trash.len()), (1, 1));
            for item in document
                .connections
                .iter()
                .chain(document.trash.iter().map(|entry| &entry.connection))
            {
                let secrets = target.decrypt_stored(item).unwrap();
                assert!(secrets.oci.is_none());
                assert_eq!(
                    crate::oci::Credential::from_modules(item, &secrets).is_ok(),
                    has_key
                );
            }
        }
    }
}
