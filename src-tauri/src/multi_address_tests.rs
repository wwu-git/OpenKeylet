use super::*;
use serde_json::json;

fn input(name: &str, kind: &str, value: &str, addresses: &[&str]) -> ConnectionInput {
    serde_json::from_value(json!({"name":name,"itemType":"custom","modules":[
        {"kind":kind,"value":value,"alternateValues":addresses}
    ]})).unwrap()
}

#[test]
fn other_addresses_survive_save_reopen_and_default_change() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("vault");
    let vault = Vault::open(path.clone()).unwrap();
    let saved = vault.save_connection(input("hosts", "host", " A.EXAMPLE ", &["[::1]", "b.example"])).unwrap();
    assert_eq!(saved.host, "a.example");
    assert_eq!(saved.modules[0].alternate_values, ["::1", "b.example"]);
    drop(vault);
    let vault = Vault::open(path).unwrap();
    let old = vault.get_connection(saved.id).unwrap();
    assert_eq!(old.stored.modules[0].alternate_values, ["::1", "b.example"]);
    let mut next: ConnectionInput = serde_json::from_value(serde_json::to_value(old.stored.portable()).unwrap()).unwrap();
    next.modules[0].value = "b.example".into();
    next.modules[0].alternate_values = vec!["a.example".into(), "::1".into()];
    let updated = vault.save_connection(next).unwrap();
    assert_eq!(updated.host, "b.example");
    assert_eq!(updated.modules[0].alternate_values, ["a.example", "::1"]);
}

#[test]
fn validates_every_address_and_preserves_saved_data_on_failure() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    let saved = vault.save_connection(input("URLs", "url", "https://a.example/", &["https://b.example/"])).unwrap();
    let before = std::fs::read(directory.path().join("vault.json")).unwrap();
    for (kind, default, addresses) in [
        ("url", "https://a.example/", vec!["file:///tmp/test"]),
        ("url", "https://a.example/", vec!["https://user:pass@b.example/"]),
        ("url", "https://a.example/", vec!["https://a.example"]),
        ("host", "a.example", vec!["A.EXAMPLE"]),
        ("host", "a.example", vec!["bad host"]),
        ("host", "a.example", vec![" "]),
        ("url", "", vec!["https://b.example/"]),
        ("port", "22", vec!["23"]),
        ("password", "", vec!["must-not-store"]),
    ] {
        let mut invalid = input("URLs", kind, default, &addresses);
        invalid.id = Some(saved.id);
        assert!(vault.save_connection(invalid).is_err(), "{kind} accepted invalid addresses");
        assert_eq!(std::fs::read(directory.path().join("vault.json")).unwrap(), before);
    }
    let mut auto = input("GitHub", "url", "", &["https://b.example/"]);
    auto.http_auth_type = "auto".into();
    auto.modules.push(serde_json::from_value(json!({"kind":"apiCredential"})).unwrap());
    auto.secrets.named_secrets.insert("apiCredential".into(), "synthetic".into());
    assert!(vault.save_connection(auto).is_err());
    assert!(vault.save_connection(input("empty", "host", "", &[])).is_ok());
}

#[test]
fn version14_single_address_remains_readable_and_rewrites_as18() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    let saved = vault.save_connection(input("legacy", "host", "a.example", &[])).unwrap();
    let path = directory.path().join("vault.json");
    let mut document: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    document["version"] = json!(14);
    assert!(document["connections"][0]["modules"][0].get("alternateValues").is_none());
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    drop(vault);
    let vault = Vault::open(directory.path().into()).unwrap();
    assert_eq!(vault.get_connection(saved.id).unwrap().stored.host, "a.example");
    assert!(vault.get_connection(saved.id).unwrap().stored.modules[0].alternate_values.is_empty());
    vault.save_connection(input("new", "host", "b.example", &["c.example"])).unwrap();
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(document["version"], 18);
}

#[test]
fn backups_preserve_default_and_other_addresses_and_restore_idempotently() {
    let directory = tempfile::tempdir().unwrap();
    let source = Vault::open(directory.path().join("source")).unwrap();
    let saved = source.save_connection(input("services", "url", "https://a.example/", &["https://b.example/"])).unwrap();
    let snapshot = crate::backup::cloud_snapshot(&source).unwrap();
    let target = Vault::open(directory.path().join("target")).unwrap();
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &snapshot).unwrap().added, 1);
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &snapshot).unwrap().added, 0);
    assert_eq!(target.list_connections().unwrap()[0].modules[0].alternate_values, ["https://b.example/"]);
    let file = directory.path().join("addresses.mvault");
    crate::backup::export_protected(&source, &file, "synthetic-backup-password").unwrap();
    let file_target = Vault::open(directory.path().join("file-target")).unwrap();
    assert_eq!(crate::backup::import_with_password(&file_target, &file, "synthetic-backup-password").unwrap().added, 1);
    assert_eq!(file_target.list_connections().unwrap()[0].modules[0].alternate_values, ["https://b.example/"]);
    let old = source.get_connection(saved.id).unwrap();
    let mut next: ConnectionInput = serde_json::from_value(serde_json::to_value(old.stored.portable()).unwrap()).unwrap();
    next.modules[0].value = "https://b.example/".into();
    next.modules[0].alternate_values = vec!["https://a.example/".into()];
    source.save_connection(next).unwrap();
    let changed = crate::backup::cloud_snapshot(&source).unwrap();
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &changed).unwrap().added, 1);
    assert_eq!(target.list_connections().unwrap().len(), 2);
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &changed).unwrap().added, 0);
    assert_eq!(target.list_connections().unwrap().len(), 2);
    assert!(target.list_connections().unwrap().iter().any(|item| item.base_url == "https://b.example/" && item.modules[0].alternate_values == ["https://a.example/"]));
    let mut legacy: serde_json::Value = serde_json::from_slice(&snapshot).unwrap();
    legacy["version"] = json!(8);
    legacy["connections"][0]["connection"]["modules"][0].as_object_mut().unwrap().remove("alternateValues");
    let legacy_target = Vault::open(directory.path().join("legacy-cloud")).unwrap();
    assert_eq!(crate::backup::restore_cloud_snapshot(&legacy_target, &serde_json::to_vec(&legacy).unwrap()).unwrap().added, 1);
    assert!(legacy_target.list_connections().unwrap()[0].modules[0].alternate_values.is_empty());
}

#[test]
fn oci_addresses_use_standard_modules_and_requests_validate_targets() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().join("source")).unwrap();
    let saved = vault.save_oci_connection(serde_json::from_value(json!({"name":"OCI"})).unwrap(), None, crate::oci::tests::config(), Some(crate::oci::tests::KEY), None).unwrap();
    let old = vault.get_connection(saved.id).unwrap();
    let mut input: ConnectionInput = serde_json::from_value(serde_json::to_value(old.stored.portable()).unwrap()).unwrap();
    input.modules.iter_mut().find(|module| module.kind == "url").unwrap().alternate_values = vec!["https://other.example/".into()];
    vault.save_connection(input).unwrap();
    let target = Vault::open(directory.path().join("target")).unwrap();
    crate::backup::restore_cloud_snapshot(&target, &crate::backup::cloud_snapshot(&vault).unwrap()).unwrap();
    let restored = target.get_connection(saved.id).unwrap();
    assert_eq!(restored.stored.modules.iter().find(|module| module.kind == "url").unwrap().alternate_values, ["https://other.example/"]);
    let request = reqwest::Request::new(reqwest::Method::GET, url::Url::parse("https://other.example/").unwrap());
    assert!(crate::oci::validate_request(&crate::oci::tests::config(), &request).is_err());
}

#[test]
fn other_address_only_changes_make_conflicts_but_legacy_spelling_does_not() {
    let directory = tempfile::tempdir().unwrap();
    let source = Vault::open(directory.path().join("source")).unwrap();
    let saved = source.save_connection(input("services", "url", "https://api.example.test/", &["https://b.example/"])).unwrap();
    let snapshot = crate::backup::cloud_snapshot(&source).unwrap();
    let target = Vault::open(directory.path().join("target")).unwrap();
    crate::backup::restore_cloud_snapshot(&target, &snapshot).unwrap();
    let old = source.get_connection(saved.id).unwrap();
    let mut next: ConnectionInput = serde_json::from_value(serde_json::to_value(old.stored.portable()).unwrap()).unwrap();
    next.modules[0].alternate_values = vec!["https://c.example/".into()];
    source.save_connection(next).unwrap();
    let changed = crate::backup::cloud_snapshot(&source).unwrap();
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &changed).unwrap().added, 1);
    assert_eq!(crate::backup::restore_cloud_snapshot(&target, &changed).unwrap().added, 0);
    assert_eq!(target.list_connections().unwrap().len(), 2);
    let legacy = Vault::open(directory.path().join("legacy")).unwrap();
    let single = legacy.save_connection(input("legacy single", "url", "https://api.example.test/", &[])).unwrap();
    let mut snapshot: serde_json::Value = serde_json::from_slice(&crate::backup::cloud_snapshot(&legacy).unwrap()).unwrap();
    snapshot["version"] = json!(8);
    snapshot["connections"][0]["connection"]["modules"][0]["value"] = json!("https://api.example.test");
    let path = directory.path().join("legacy/vault.json");
    let mut local: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    local["version"] = json!(14);
    local["connections"][0]["modules"][0]["value"] = json!("https://api.example.test");
    std::fs::write(&path, serde_json::to_vec(&local).unwrap()).unwrap();
    assert_eq!(crate::backup::restore_cloud_snapshot(&legacy, &serde_json::to_vec(&snapshot).unwrap()).unwrap().added, 0);
    assert_eq!(legacy.list_connections().unwrap().len(), 1);
    assert_eq!(legacy.list_connections().unwrap()[0].id, single.id);
}
