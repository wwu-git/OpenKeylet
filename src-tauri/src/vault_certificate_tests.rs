use super::*;
use crate::{
    certificates::{CertificateKind, CertificateRequest},
    model::{CertificateAuthorityBinding, CertificateAuthorityKind},
};

fn key(seed: u8) -> russh::keys::PrivateKey {
    russh::keys::PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
        &[seed; 32],
    ))
}
fn fixture(vault: &Vault) -> Uuid {
    let input:ConnectionInput=serde_json::from_value(serde_json::json!({"name":"Synthetic CA","itemType":"custom","modules":[{"kind":"privateKey"}],"secrets":{"privateKey":key(41).to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().to_string()}})).unwrap();
    let item = vault.save_connection(input).unwrap();
    let binding = CertificateAuthorityBinding {
        kind: CertificateAuthorityKind::Ssh,
        private_key: item.modules[0].id,
        ca_certificate: None,
    };
    vault
        .set_certificate_authority(item.id, Some(binding))
        .unwrap();
    item.id
}
fn request() -> CertificateRequest {
    CertificateRequest::SshUser {
        public_key: key(42).public_key().to_openssh().unwrap(),
        principals: vec!["test-user".into()],
        key_id: "simulation".into(),
        extensions: vec![],
    }
}
fn allow(v: &Vault, id: Uuid) {
    v.change_signing_permissions(SigningPermissionChange::Allow {
        item_id: id,
        kinds: vec![CertificateKind::SshUser],
        max_ttl_seconds: 600,
    })
    .unwrap();
    v.change_signing_permissions(SigningPermissionChange::Enable(true))
        .unwrap();
}

#[test]
fn duplicate_connection_rejects_certificate_authority_items() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let id = fixture(&vault);
    let source = vault.get_connection(id).unwrap();
    assert!(vault.duplicate_connection(id, &source.stored.updated_at).is_err());
    assert_eq!(vault.list_connections().unwrap().len(), 1);
}

#[test]
fn authority_default_deny_grants_and_live_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path().into()).unwrap();
    let id = fixture(&v);
    let rid = Uuid::new_v4();
    assert!(!v.certificate_capabilities().unwrap().enabled);
    assert!(v.agent_connection(id).is_err());
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
            .is_err()
    );
    allow(&v, id);
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 601)
            .is_err()
    );
    let first = v
        .issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
        .unwrap();
    assert_eq!(
        first,
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
            .unwrap()
    );
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 61)
            .is_err()
    );
    v.change_signing_permissions(SigningPermissionChange::Remove(id))
        .unwrap();
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
            .is_err()
    );
    assert_eq!(v.read_document().unwrap().certificate_issuances.len(), 1);
    let raw = fs::read_to_string(dir.path().join("vault.json")).unwrap();
    assert!(!raw.contains("BEGIN OPENSSH PRIVATE KEY"));
    assert!(!raw.contains(&first.certificate));
}
#[test]
fn replacement_incomplete_and_backup_preserve_restriction_without_grants() {
    let dir = tempfile::tempdir().unwrap();
    let v = Vault::open(dir.path().into()).unwrap();
    let id = fixture(&v);
    allow(&v, id);
    let request_id = Uuid::new_v4();
    v.issue_certificate(CertificateActor::Owner, id, request_id, request(), 60)
        .unwrap();
    let mut edit: ConnectionInput = serde_json::from_value(
        serde_json::to_value(v.get_connection(id).unwrap().stored.portable()).unwrap(),
    )
    .unwrap();
    edit.secrets.private_key = Some(
        key(43)
            .to_openssh(russh::keys::ssh_key::LineEnding::LF)
            .unwrap()
            .to_string(),
    );
    edit.certificate_authority = None;
    v.save_connection(edit.clone()).unwrap();
    assert!(!v.certificate_capabilities().unwrap().authorities[0].effective);
    assert!(
        v.issue_certificate(CertificateActor::Owner, id, request_id, request(), 60)
            .is_err()
    );
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, Uuid::new_v4(), request(), 60)
            .is_err()
    );
    let d = tempfile::tempdir().unwrap();
    let restored = Vault::open(d.path().into()).unwrap();
    let portable = v.export_connections().unwrap();
    assert_eq!(
        restored.merge_connections(portable.clone()).unwrap().added,
        1
    );
    assert_eq!(restored.merge_connections(portable).unwrap().merged, 1);
    assert!(restored.agent_connection(id).is_err());
    assert!(
        restored
            .read_document()
            .unwrap()
            .certificate_issuances
            .is_empty()
    );
    assert!(
        restored
            .certificate_capabilities()
            .unwrap()
            .grants
            .is_empty()
    );
    edit.modules = vec![ItemModule {
        kind: "publicKey".into(),
        value: key(43).public_key().to_openssh().unwrap(),
        ..Default::default()
    }];
    edit.item_type = "custom".into();
    v.save_connection(edit).unwrap();
    assert!(v.agent_connection(id).is_err());
    assert!(!v.certificate_capabilities().unwrap().authorities[0].configured);
    v.set_certificate_authority(id, None).unwrap();
    assert!(v.agent_connection(id).is_ok());
}
#[test]
fn migration_v16_only_defaults_and_host_trust_kind_defaults() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::open(d.path().into()).unwrap();
    let id = fixture(&v);
    let mut raw: serde_json::Value =
        serde_json::from_slice(&fs::read(&v.inner.vault_path).unwrap()).unwrap();
    raw["version"] = 16.into();
    raw.as_object_mut().unwrap().remove("certificateIssuances");
    raw["settings"]
        .as_object_mut()
        .unwrap()
        .remove("agentSigningEnabled");
    fs::write(&v.inner.vault_path, serde_json::to_vec(&raw).unwrap()).unwrap();
    drop(v);
    let v = Vault::open(d.path().into()).unwrap();
    assert_eq!(v.read_document().unwrap().version, 18);
    assert!(v.agent_connection(id).is_err());
    assert!(!fs::read_dir(d.path()).unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("before-oci")
    }));
    let trust: crate::model::SshHostTrust = serde_json::from_value(
        serde_json::json!({"itemId":id,"host":"localhost","port":22,"fingerprint":"fake"}),
    )
    .unwrap();
    assert_eq!(trust.kind, crate::model::SshHostTrustKind::HostKey);
}
#[test]
fn concurrent_replay_commits_once() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::open(d.path().into()).unwrap();
    let id = fixture(&v);
    allow(&v, id);
    let rid = Uuid::new_v4();
    let threads: Vec<_> = (0..4)
        .map(|_| {
            let v = v.clone();
            std::thread::spawn(move || {
                v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
                    .unwrap()
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results.iter().all(|r| r == &results[0]));
    assert_eq!(v.read_document().unwrap().certificate_issuances.len(), 1);
    drop(v);
    let reopened = Vault::open(d.path().into()).unwrap();
    assert_eq!(
        results[0],
        reopened
            .issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
            .unwrap()
    );
}

#[test]
fn malformed_binding_and_wrong_kind_cannot_sign_and_request_payload_conflicts() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::open(d.path().into()).unwrap();
    let id = fixture(&v);
    allow(&v, id);
    assert!(
        v.set_certificate_authority(
            id,
            Some(CertificateAuthorityBinding {
                kind: CertificateAuthorityKind::Ssh,
                private_key: Uuid::new_v4(),
                ca_certificate: None
            })
        )
        .is_err()
    );
    assert!(
        v.change_signing_permissions(SigningPermissionChange::Allow {
            item_id: id,
            kinds: vec![CertificateKind::TlsServer],
            max_ttl_seconds: 60
        })
        .is_err()
    );
    assert!(
        v.change_signing_permissions(SigningPermissionChange::Allow {
            item_id: id,
            kinds: vec![],
            max_ttl_seconds: 60
        })
        .is_err()
    );
    let rid = Uuid::new_v4();
    v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
        .unwrap();
    let mut changed = request();
    if let CertificateRequest::SshUser { principals, .. } = &mut changed {
        principals.push("other".into());
    }
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, changed, 60)
            .is_err()
    );
    let mut empty = request();
    if let CertificateRequest::SshUser { principals, .. } = &mut empty {
        principals.clear();
    }
    assert!(
        v.issue_certificate(CertificateActor::Owner, id, Uuid::new_v4(), empty, 60)
            .is_err()
    );
    v.change_signing_permissions(SigningPermissionChange::Enable(false))
        .unwrap();
    assert!(
        v.issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
            .is_err()
    );
    assert!(
        v.issue_certificate(CertificateActor::Owner, id, rid, request(), 60)
            .is_ok()
    );
}

#[test]
fn normalized_replay_and_portable_module_id_remapping() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::open(d.path().into()).unwrap();
    let id = fixture(&v);
    allow(&v, id);
    let rid = Uuid::new_v4();
    let issued = v
        .issue_certificate(CertificateActor::Agent, id, rid, request(), 60)
        .unwrap();
    let mut req = request();
    if let CertificateRequest::SshUser {
        public_key,
        principals,
        key_id,
        ..
    } = &mut req
    {
        *public_key = format!("  {public_key}\n");
        principals.push(" test-user ".into());
        *key_id = " simulation ".into();
    }
    assert_eq!(
        issued,
        v.issue_certificate(CertificateActor::Agent, id, rid, req, 60)
            .unwrap()
    );
    let mut entries = v.export_connections().unwrap();
    let portable = &mut entries[0].0;
    let mapped = Uuid::new_v4();
    portable.modules[0].id = mapped;
    portable.certificate_authority.as_mut().unwrap().private_key = mapped;
    assert_eq!(v.merge_connections(entries).unwrap().merged, 1);
    let owner = v.get_connection(id).unwrap();
    let mismatch = v
        .check_item_key(id, None, &key(45).public_key().to_openssh().unwrap())
        .unwrap();
    assert!(!mismatch.matches);
    assert!(
        v.check_item_key(
            id,
            Some(Uuid::new_v4()),
            &key(41).public_key().to_openssh().unwrap()
        )
        .is_err()
    );
    assert_eq!(
        owner.stored.certificate_authority.unwrap().private_key,
        owner.stored.modules[0].id
    );
}

#[test]
fn tls_certificate_rotation_invalidates_pinned_grant() {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let key = KeyPair::generate().unwrap();
    let mut params = CertificateParams::new(vec!["ca.example.test".into()]).unwrap();
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
    let cert = params.self_signed(&key).unwrap();
    let input:ConnectionInput=serde_json::from_value(serde_json::json!({"name":"Synthetic TLS CA","itemType":"custom","modules":[{"kind":"privateKey"},{"kind":"caCertificate","value":cert.pem()}],"secrets":{"privateKey":key.serialize_pem()}})).unwrap();
    let d = tempfile::tempdir().unwrap();
    let v = Vault::open(d.path().into()).unwrap();
    let item = v.save_connection(input).unwrap();
    v.set_certificate_authority(
        item.id,
        Some(CertificateAuthorityBinding {
            kind: CertificateAuthorityKind::Tls,
            private_key: item.modules[0].id,
            ca_certificate: Some(item.modules[1].id),
        }),
    )
    .unwrap();
    v.change_signing_permissions(SigningPermissionChange::Allow {
        item_id: item.id,
        kinds: vec![CertificateKind::TlsServer],
        max_ttl_seconds: 60,
    })
    .unwrap();
    v.change_signing_permissions(SigningPermissionChange::Enable(true))
        .unwrap();
    assert!(v.certificate_capabilities().unwrap().authorities[0].effective);
    params.serial_number = Some(42u64.into());
    let rotated = params.self_signed(&key).unwrap();
    let mut edit: ConnectionInput = serde_json::from_value(
        serde_json::to_value(v.get_connection(item.id).unwrap().stored.portable()).unwrap(),
    )
    .unwrap();
    edit.modules[1].value = rotated.pem();
    v.save_connection(edit).unwrap();
    assert!(!v.certificate_capabilities().unwrap().authorities[0].effective);
    let leaf_key = KeyPair::generate().unwrap();
    let csr = CertificateParams::new(vec!["service.example.test".into()])
        .unwrap()
        .serialize_request(&leaf_key)
        .unwrap();
    assert!(
        v.issue_certificate(
            CertificateActor::Agent,
            item.id,
            Uuid::new_v4(),
            CertificateRequest::TlsServer {
                csr: csr.pem().unwrap(),
                names: vec!["service.example.test".into()]
            },
            60
        )
        .is_err()
    );
}
