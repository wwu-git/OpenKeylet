use super::*;
use crate::model::AwsModuleBindings;
use serde_json::json;

fn input() -> ConnectionInput {
    let ids: Vec<_> = (0..5).map(|_| Uuid::new_v4()).collect();
    serde_json::from_value(json!({
        "name":"Synthetic AWS", "itemType":"api", "folder":"agent-test", "httpAuthType":"aws-sigv4",
        "awsBindings":{"serviceUrl":ids[0],"region":ids[1],"accessKeyId":ids[2],"secretAccessKey":ids[3],"sessionToken":ids[4]},
        "modules":[
            {"id":ids[0],"kind":"url","value":"https://ec2.us-east-1.amazonaws.com/","agentVisible":true},
            {"id":ids[1],"kind":"customSecret","name":"region","agentVisible":true},
            {"id":ids[2],"kind":"customSecret","name":"access id","agentVisible":false},
            {"id":ids[3],"kind":"customSecret","name":"secret key","agentVisible":false},
            {"id":ids[4],"kind":"customSecret","name":"session token","agentVisible":false}
        ],
        "secrets":{"namedSecrets":{"region":"us-east-1","access id":"AKIDSYNTHETIC","secret key":"synthetic-aws-secret"}}
    })).unwrap()
}

#[test]
fn provider_bindings_survive_rename_reorder_and_backups_without_rebinding() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().join("source")).unwrap();
    let original = input();
    let bindings = original.aws_bindings.clone();
    let saved = vault.save_connection(original).unwrap();
    let mut edit: ConnectionInput = serde_json::from_value(
        serde_json::to_value(vault.get_connection(saved.id).unwrap().stored.portable()).unwrap(),
    )
    .unwrap();
    edit.modules[2].name = "renamed access id".into();
    edit.secrets
        .named_secrets
        .insert("renamed access id".into(), "AKIDSYNTHETIC".into());
    edit.modules.reverse();
    vault.save_connection(edit).unwrap();
    let connection = vault.get_connection(saved.id).unwrap();
    assert_eq!(connection.stored.aws_bindings, bindings);
    assert!(crate::aws::Credential::from_modules(&connection.stored, &connection.secrets).is_ok());
    let restored = Vault::open(dir.path().join("restored")).unwrap();
    let snapshot = crate::backup::cloud_snapshot(&vault).unwrap();
    assert_eq!(
        crate::backup::restore_cloud_snapshot(&restored, &snapshot)
            .unwrap()
            .added,
        1
    );
    assert_eq!(
        crate::backup::restore_cloud_snapshot(&restored, &snapshot)
            .unwrap()
            .added,
        0
    );
    assert_eq!(
        restored
            .get_connection(saved.id)
            .unwrap()
            .stored
            .aws_bindings,
        bindings
    );
    let mut edit: ConnectionInput =
        serde_json::from_value(serde_json::to_value(connection.stored.portable()).unwrap())
            .unwrap();
    edit.modules
        .retain(|module| Some(module.id) != bindings.as_ref().unwrap().secret_access_key);
    vault.save_connection(edit).unwrap();
    let incomplete = vault.get_connection(saved.id).unwrap();
    assert!(crate::aws::Credential::from_modules(&incomplete.stored, &incomplete.secrets).is_err());
    assert_eq!(incomplete.stored.http_auth_type, "aws-sigv4");
    assert!(
        incomplete
            .stored
            .aws_bindings
            .unwrap()
            .secret_access_key
            .is_none()
    );
}

#[test]
fn provider_schema17_upgrade_preserves_existing_data_without_oci_migration() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    vault.save_connection(input()).unwrap();
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(&vault.inner.vault_path).unwrap()).unwrap();
    raw["version"] = 17.into();
    raw["connections"][0]
        .as_object_mut()
        .unwrap()
        .remove("awsBindings");
    fs::write(&vault.inner.vault_path, serde_json::to_vec(&raw).unwrap()).unwrap();
    drop(vault);
    let reopened = Vault::open(dir.path().into()).unwrap();
    let mut actual: serde_json::Value =
        serde_json::from_slice(&fs::read(&reopened.inner.vault_path).unwrap()).unwrap();
    assert_eq!(actual["version"], 18);
    actual["version"] = 17.into();
    assert_eq!(actual, raw);
    assert!(!fs::read_dir(dir.path()).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("before-oci")
    }));
}

#[test]
fn provider_semantic_binding_dedup_uses_module_positions() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let saved = vault.save_connection(input()).unwrap();
    let mut portable = vault.export_connections().unwrap();
    let item = &mut portable[0].0;
    let old = item
        .modules
        .iter()
        .map(|module| module.id)
        .collect::<Vec<_>>();
    for module in &mut item.modules {
        module.id = Uuid::new_v4();
    }
    let binding = item.aws_bindings.as_mut().unwrap();
    for role in [
        &mut binding.service_url,
        &mut binding.region,
        &mut binding.access_key_id,
        &mut binding.secret_access_key,
        &mut binding.session_token,
    ] {
        *role = role.and_then(|id| {
            old.iter()
                .position(|old| *old == id)
                .map(|index| item.modules[index].id)
        });
    }
    assert_eq!(vault.merge_connections(portable).unwrap().added, 0);
    assert_eq!(vault.list_connections().unwrap().len(), 1);
    assert_eq!(vault.list_connections().unwrap()[0].id, saved.id);
}

#[test]
fn provider_mcp_binding_patch_preserves_omissions_and_write_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let saved = vault.save_connection(input()).unwrap();
    let patch = |revision, bindings| {
        serde_json::from_value(
            json!({"id":saved.id,"expectedRevision":revision,"awsBindings":bindings}),
        )
        .unwrap()
    };
    assert!(
        vault
            .agent_update(patch(saved.revision, json!({"sessionToken":null})))
            .is_err()
    );
    vault
        .change_write_permissions(WritePermissionChange::AddFolder("agent-test"))
        .unwrap();
    vault
        .change_write_permissions(WritePermissionChange::Enable(true))
        .unwrap();
    let changed = vault
        .agent_update(patch(saved.revision, json!({"sessionToken":null})))
        .unwrap();
    let connection = vault.get_connection(saved.id).unwrap();
    let mut expected = saved.aws_bindings.unwrap();
    expected.session_token = None;
    assert_eq!(connection.stored.aws_bindings, Some(expected));
    assert!(
        vault
            .agent_update(patch(saved.revision, json!({"region":null})))
            .is_err()
    );
    let bad: AwsModuleBindings =
        serde_json::from_value(json!({"serviceUrl":Uuid::new_v4()})).unwrap();
    assert!(
        vault
            .agent_update(patch(changed.revision, json!(bad)))
            .is_err()
    );
    let empty: ConnectionInput=serde_json::from_value(json!({"name":"Incomplete AWS", "itemType":"api", "httpAuthType":"aws-sigv4", "modules":[]})).unwrap();
    let incomplete = vault.save_connection(empty).unwrap();
    assert_eq!(incomplete.http_auth_type, "aws-sigv4");
    assert!(!incomplete.can_test);
}
