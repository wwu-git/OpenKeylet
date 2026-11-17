use super::*;
use serde_json::json;

fn fixture() -> (tempfile::TempDir, Vault) {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().join("vault")).unwrap();
    (dir, vault)
}
fn allow(vault: &Vault, folders: &[&str]) {
    vault
        .update_settings_patch(
            serde_json::from_value(json!({"agentWriteEnabled":true,"agentWriteFolders":folders}))
                .unwrap(),
        )
        .unwrap();
}
fn create(vault: &Vault, name: &str, folder: &str) -> Result<WriteOutput> {
    vault.agent_create(serde_json::from_value(json!({"name":name,"itemType":"ssh","folder":folder,
        "sshAuthType":"password", "modules":[{"kind":"host","value":"a.example"},{"kind":"port","value":22},
        {"kind":"username","value":"deploy"},{"kind":"password","value":"synthetic-secret"}]})).unwrap())
}
fn update(vault: &Vault, item: &WriteOutput, patch: serde_json::Value) -> Result<WriteOutput> {
    let mut value = patch;
    value["id"] = json!(item.id);
    value["expectedRevision"] = json!(item.revision);
    vault.agent_update(serde_json::from_value(value).unwrap())
}
#[test]
fn policy_atomic_patch_and_manual_boundary() {
    let (_dir, vault) = fixture();
    assert!(create(&vault, "first", "Agent").is_err());
    allow(&vault, &["Agent"]);
    assert!(create(&vault, "child", "Agent/child").is_err());
    let first = create(&vault, "first", "Agent").unwrap();
    assert!(update(&vault, &first, json!({"folder":"Personal"})).is_err());
    let old = vault.get_connection(first.id.parse().unwrap()).unwrap();
    let host = old
        .stored
        .modules
        .iter()
        .find(|m| m.kind == "host")
        .unwrap()
        .id;
    let password = old
        .stored
        .modules
        .iter()
        .find(|m| m.kind == "password")
        .unwrap()
        .id;
    let next = update(
        &vault,
        &first,
        json!({"modules":[{"id":host,"value":"b.example"},{"id":password,"value":"replacement"}]}),
    )
    .unwrap();
    assert_eq!(next.revision, first.revision + 1);
    assert!(update(&vault, &first, json!({"description":"stale"})).is_err());
    let saved = vault.get_connection(first.id.parse().unwrap()).unwrap();
    assert_eq!(saved.stored.host, "b.example");
    assert_eq!(saved.secrets.get("password"), Some("replacement"));
    assert!(
        !saved
            .stored
            .modules
            .iter()
            .find(|m| m.id == password)
            .unwrap()
            .agent_visible()
    );
    let before = std::fs::read(vault.data_dir().join("vault.json")).unwrap();
    assert!(
        update(
            &vault,
            &next,
            json!({"description":"must rollback","modules":[{"kind":"port","value":0}]})
        )
        .is_err()
    );
    assert_eq!(
        before,
        std::fs::read(vault.data_dir().join("vault.json")).unwrap()
    );
    allow(&vault, &["Agent", "Personal"]);
    let moved = update(&vault, &next, json!({"folder":"Personal"})).unwrap();
    allow(&vault, &["Agent"]);
    assert!(update(&vault, &moved, json!({"folder":"Agent"})).is_err());
    // Desktop writes remain independent from MCP policy.
    let input = serde_json::from_value(json!({"name":"manual","folder":"Personal","modules":[{"kind":"host","value":"manual.example"}]})).unwrap();
    assert!(vault.save_connection(input).is_ok());
    let response = serde_json::to_string(&next).unwrap();
    assert!(!response.contains("replacement") && !response.contains("synthetic-secret"));
}
#[test]
fn strict_requests_and_concurrent_writes() {
    assert!(
        serde_json::from_value::<CreateInput>(
            json!({"name":"x","itemType":"ssh","folder":"Agent","agentVisible":true})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<ModulePatch>(
            json!({"kind":"password","value":"x","agentVisible":true})
        )
        .is_err()
    );
    let (_dir, vault) = fixture();
    allow(&vault, &["Agent"]);
    let first = create(&vault, "one", "Agent").unwrap();
    let a = vault.clone();
    let b = vault.clone();
    let f = first.clone();
    let g = first.clone();
    let left = std::thread::spawn(move || update(&a, &f, json!({"description":"left"})));
    let right = std::thread::spawn(move || update(&b, &g, json!({"description":"right"})));
    assert_ne!(left.join().unwrap().is_ok(), right.join().unwrap().is_ok());
}

fn key_pair() -> (String, String) {
    use russh::keys::ssh_key::{LineEnding, private::Ed25519Keypair};
    let key = russh::keys::PrivateKey::from(Ed25519Keypair::from_seed(&[7u8; 32]));
    // Use ssh-key's re-exported rand_core implementation through its encryption API.
    (
        key.to_openssh(LineEnding::LF).unwrap().to_string(),
        key.public_key().to_openssh().unwrap(),
    )
}
#[test]
fn private_key_validation_and_clear_old_passphrase() {
    let (_dir, vault) = fixture();
    allow(&vault, &["Agent"]);
    let first = create(&vault, "keys", "Agent").unwrap();
    let (key, _) = key_pair();
    let added = update(
        &vault,
        &first,
        json!({"sshAuthType":"privateKey","modules":[{"kind":"privateKey","value":key}]}),
    )
    .unwrap();
    assert_eq!(
        vault
            .get_connection(first.id.parse().unwrap())
            .unwrap()
            .stored
            .ssh_auth_type,
        "privateKey"
    );
    let before = std::fs::read(vault.data_dir().join("vault.json")).unwrap();
    let id = vault
        .get_connection(first.id.parse().unwrap())
        .unwrap()
        .stored
        .modules
        .iter()
        .find(|m| m.kind == "privateKey")
        .unwrap()
        .id;
    let error = update(
        &vault,
        &added,
        json!({"description":"rollback","modules":[{"id":id,"value":"NOT-A-KEY-SECRET"}]}),
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!error.contains("NOT-A-KEY-SECRET"));
    assert_eq!(
        before,
        std::fs::read(vault.data_dir().join("vault.json")).unwrap()
    );
    let next = update(
        &vault,
        &added,
        json!({"modules":[{"id":id,"value":key},{"kind":"passphrase","value":"old-passphrase"}]}),
    )
    .unwrap();
    assert!(
        vault
            .get_connection(first.id.parse().unwrap())
            .unwrap()
            .secrets
            .get("passphrase")
            .is_none()
    );
    assert!(
        update(
            &vault,
            &next,
            json!({"modules":[{"kind":"passphrase","value":"bad"}]})
        )
        .is_err()
    );
}
#[test]
fn oci_ordinary_patch_preserves_signing_data() {
    let (_dir, vault) = fixture();
    allow(&vault, &["Agent"]);
    let input =
        serde_json::from_value(json!({"name":"OCI","folder":"Agent","modules":[]})).unwrap();
    let item = vault
        .save_oci_connection(
            input,
            None,
            crate::oci::tests::config(),
            Some(crate::oci::tests::KEY),
            None,
        )
        .unwrap();
    let before = vault.get_connection(item.id).unwrap();
    let config = serde_json::to_value(&before.secrets.oci).unwrap();
    let output: WriteOutput = item.into();
    let next=update(&vault,&output,json!({"description":"ordinary","modules":[{"kind":"customSecret","name":"region","value":"just metadata"}]})).unwrap();
    let after = vault.get_connection(output.id.parse().unwrap()).unwrap();
    assert_eq!(serde_json::to_value(&after.secrets.oci).unwrap(), config);
    assert_eq!(after.stored.base_url, before.stored.base_url);
    for patch in [
        json!({"httpAuthType":"bearer"}),
        json!({"itemType":"ssh"}),
        json!({"modules":[{"id":after.stored.modules[0].id,"value":"https://evil.invalid"}]}),
    ] {
        assert!(update(&vault, &next, patch).is_err());
    }
    assert_eq!(
        vault
            .get_connection(output.id.parse().unwrap())
            .unwrap()
            .stored
            .revision,
        next.revision
    );
}
#[test]
fn custom_modules_cross_item_ids_and_api_modes() {
    let (_dir, vault) = fixture();
    allow(&vault, &["Agent"]);
    let first = create(&vault, "first", "Agent").unwrap();
    let other = create(&vault, "other", "Agent").unwrap();
    assert!(
        update(
            &vault,
            &first,
            json!({"modules":[{"id":other.module_ids[0],"value":"wrong"}]})
        )
        .is_err()
    );
    assert!(
        update(
            &vault,
            &first,
            json!({"modules":[{"kind":"customSecret","name":"password","value":"bad"}]})
        )
        .is_err()
    );
    let next = update(
        &vault,
        &first,
        json!({"modules":[{"kind":"customSecret","name":"team","value":"a"}]}),
    )
    .unwrap();
    let module = next.module_ids.last().unwrap();
    let next = update(
        &vault,
        &next,
        json!({"modules":[{"id":module,"name":"team_id","value":"b"}]}),
    )
    .unwrap();
    let saved = vault.get_connection(first.id.parse().unwrap()).unwrap();
    assert_eq!(saved.secrets.get("team_id"), Some("b"));
    assert!(saved.secrets.get("team").is_none());
    assert_eq!(next.module_ids.last().unwrap(), module);
    for mode in ["bearer", "apiKey", "basic"] {
        let created=vault.agent_create(serde_json::from_value(json!({"name":mode,"itemType":"api","folder":"Agent","httpAuthType":mode,"authHeader":"X-API-Key","modules":[{"kind":"url","value":"https://api.example/v1"},{"kind":"apiCredential","value":"key"},{"kind":"username","value":"user"},{"kind":"password","value":"pw"}]})).unwrap()).unwrap();
        assert_eq!(
            vault
                .get_connection(created.id.parse().unwrap())
                .unwrap()
                .stored
                .http_auth_type,
            mode
        );
    }
}

#[test]
fn encrypted_key_requires_same_request_passphrase_and_rolls_back() {
    use russh::keys::ssh_key::{
        LineEnding,
        private::Ed25519Keypair,
        rand_core::{TryCryptoRng, TryRng},
    };
    let key = russh::keys::PrivateKey::from(Ed25519Keypair::from_seed(&[9u8; 32]));
    struct TestRng;
    impl TryRng for TestRng {
        type Error = std::convert::Infallible;
        fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
            Ok(17)
        }
        fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
            Ok(17)
        }
        fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), Self::Error> {
            bytes.fill(17);
            Ok(())
        }
    }
    impl TryCryptoRng for TestRng {}
    let encrypted = key
        .encrypt(&mut TestRng, "test-passphrase")
        .unwrap()
        .to_openssh(LineEnding::LF)
        .unwrap()
        .to_string();
    let plain = key.to_openssh(LineEnding::LF).unwrap().to_string();
    let (_dir, vault) = fixture();
    allow(&vault, &["Agent"]);
    let first = create(&vault, "encrypted", "Agent").unwrap();
    assert!(
        update(
            &vault,
            &first,
            json!({"modules":[{"kind":"privateKey","value":encrypted}]})
        )
        .is_err()
    );
    assert!(update(&vault,&first,json!({"modules":[{"kind":"privateKey","value":encrypted},{"kind":"passphrase","value":"wrong"}]})).is_err());
    let next=update(&vault,&first,json!({"sshAuthType":"privateKey","modules":[{"kind":"privateKey","value":encrypted},{"kind":"passphrase","value":"test-passphrase"}]})).unwrap();
    let stored = vault.get_connection(first.id.parse().unwrap()).unwrap();
    let key_id = stored
        .stored
        .modules
        .iter()
        .find(|m| m.kind == "privateKey")
        .unwrap()
        .id;
    let pass_id = stored
        .stored
        .modules
        .iter()
        .find(|m| m.kind == "passphrase")
        .unwrap()
        .id;
    // A previously stored correct passphrase is not sufficient for key replacement.
    assert!(
        update(
            &vault,
            &next,
            json!({"modules":[{"id":key_id,"value":encrypted}]})
        )
        .is_err()
    );
    assert!(
        update(
            &vault,
            &next,
            json!({"description":"rollback","modules":[{"id":pass_id,"value":"wrong"}]})
        )
        .is_err()
    );
    let next = update(
        &vault,
        &next,
        json!({"modules":[{"id":pass_id,"value":"test-passphrase"}]}),
    )
    .unwrap();
    update(
        &vault,
        &next,
        json!({"modules":[{"id":key_id,"value":plain}]}),
    )
    .unwrap();
    assert!(
        vault
            .get_connection(first.id.parse().unwrap())
            .unwrap()
            .secrets
            .get("passphrase")
            .is_none()
    );
}

#[test]
fn revoked_settings_manual_versions_and_legacy_ids() {
    let (_dir, vault) = fixture();
    for folders in [json!([""]), json!(["Agent", " Agent "])] {
        assert!(
            vault
                .update_settings_patch(
                    serde_json::from_value(json!({"agentWriteFolders":folders})).unwrap()
                )
                .is_err()
        );
    }
    allow(&vault, &[" Agent "]);
    assert_eq!(vault.write_capabilities().unwrap().folders, vec!["Agent"]);
    let first = create(&vault, "one", "Agent").unwrap();
    vault
        .set_connection_enabled(first.id.parse().unwrap(), false)
        .unwrap();
    assert!(update(&vault, &first, json!({"description":"stale"})).is_err());
    vault
        .update_settings_patch(serde_json::from_value(json!({"agentWriteEnabled":false})).unwrap())
        .unwrap();
    assert!(!vault.write_capabilities().unwrap().enabled);
    assert_eq!(vault.write_capabilities().unwrap().folders, vec!["Agent"]);
    assert!(create(&vault, "two", "Agent").is_err());
    let path = vault.data_dir().join("vault.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    doc["version"] = json!(12);
    doc["connections"][0]
        .as_object_mut()
        .unwrap()
        .remove("revision");
    for m in doc["connections"][0]["modules"].as_array_mut().unwrap() {
        m.as_object_mut().unwrap().remove("id");
    }
    std::fs::write(&path, serde_json::to_vec(&doc).unwrap()).unwrap();
    let a = vault.list_connections().unwrap();
    let b = vault.list_connections().unwrap();
    assert_eq!(a[0].modules[0].id, b[0].modules[0].id);
    assert_eq!(a[0].revision, 1);
}
