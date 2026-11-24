use super::*;
use crate::certificates::{CertificateKind, CertificateRequest};
use crate::{
    model::{CertificateAuthorityBinding, CertificateAuthorityKind, ConnectionInput},
    vault::{CertificateActor, SigningPermissionChange, WritePermissionChange},
};
fn key(seed: u8) -> russh::keys::PrivateKey {
    russh::keys::PrivateKey::from(russh::keys::ssh_key::private::Ed25519Keypair::from_seed(
        &[seed; 32],
    ))
}
fn setup() -> (tempfile::TempDir, VaultMcp, Uuid) {
    let d = tempfile::tempdir().unwrap();
    let vault = Vault::open(d.path().into()).unwrap();
    let input:ConnectionInput=serde_json::from_value(json!({"name":"Synthetic MCP CA","itemType":"custom","folder":"agent-test","modules":[{"kind":"privateKey","agentVisible":true},{"kind":"passphrase","agentVisible":true}],"secrets":{"privateKey":key(71).to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().to_string(),"passphrase":"synthetic-passphrase"}})).unwrap();
    let item = vault.save_connection(input).unwrap();
    vault
        .set_certificate_authority(
            item.id,
            Some(CertificateAuthorityBinding {
                kind: CertificateAuthorityKind::Ssh,
                private_key: item.modules[0].id,
                ca_certificate: None,
            }),
        )
        .unwrap();
    (d, VaultMcp::new(vault), item.id)
}
fn request() -> CertificateRequest {
    CertificateRequest::SshUser {
        public_key: key(72).public_key().to_openssh().unwrap(),
        principals: vec!["test-user".into()],
        key_id: "mcp-test".into(),
        extensions: vec![],
    }
}
fn is_error(result: &CallToolResult) -> bool {
    result.is_error.unwrap_or(false)
}
#[tokio::test]
async fn mcp_ca_search_and_all_generic_secret_paths_are_protected() {
    let (_d, server, id) = setup();
    let output = server
        .search_items(ItemsSearchInput {
            query: Some("Synthetic MCP CA".into()),
        })
        .unwrap();
    let text = serde_json::to_string(&output).unwrap();
    assert!(!text.contains("BEGIN OPENSSH PRIVATE KEY"));
    assert!(!text.contains("synthetic-passphrase"));
    assert!(
        output.items[0]
            .modules
            .iter()
            .all(|module| !module.agent_visible && module.value.is_none())
    );
    assert_eq!(
        output.items[0].actions,
        vec!["certificate_issue", "certificate_check_key"]
    );
    for capability in ["fill", "ssh", "http"] {
        assert!(
            server
                .resolve_item_for_action("Synthetic MCP CA", Some(capability))
                .is_err()
        );
    }
    assert!(server.vault.agent_secret_value(id, "privateKey").is_err());
    assert!(server.vault.agent_secret_value(id, "passphrase").is_err());
    let session = Uuid::new_v4();
    server.bind_terminal_item(session, id);
    assert!(server.resolve_terminal_item(session, "").is_err());
    let mut connection = server.vault.get_connection(id).unwrap();
    assert!(
        server
            .resolve_secret_bindings(
                &mut connection,
                &HashMap::from([("x".into(), "private key".into())]),
                ""
            )
            .is_err()
    );
    let result = server
        .terminal_run(Parameters(TerminalRunInput {
            item: "Synthetic MCP CA".into(),
            command: "echo {{openkeylet:private key}}".into(),
            stdin: None,
            cwd: None,
            timeout_seconds: Some(1),
            secret_env: HashMap::new(),
        }))
        .await;
    assert!(is_error(&result));
    let result = server
        .terminal_start(Parameters(TerminalStartInput {
            item: "Synthetic MCP CA".into(),
            program: String::new(),
            args: vec![],
            cwd: None,
            command: String::new(),
            secret_env: HashMap::from([("CA".into(), "privateKey".into())]),
        }))
        .await;
    assert!(is_error(&result));
}
#[tokio::test]
async fn mcp_issue_replays_and_rechecks_revocation_without_exposing_private_material() {
    let (_d, server, id) = setup();
    let rid = Uuid::new_v4().to_string();
    let input = || CertificateIssueInput {
        item: "Synthetic MCP CA".into(),
        request_id: rid.clone(),
        request: request(),
        ttl_seconds: 60,
    };
    assert!(is_error(
        &server.certificate_issue(Parameters(input())).await
    ));
    server
        .vault
        .change_signing_permissions(SigningPermissionChange::Allow {
            item_id: id,
            kinds: vec![CertificateKind::SshUser],
            max_ttl_seconds: 60,
        })
        .unwrap();
    server
        .vault
        .change_signing_permissions(SigningPermissionChange::Enable(true))
        .unwrap();
    let issued = server.certificate_issue(Parameters(input())).await;
    assert!(!is_error(&issued));
    let replay = server.certificate_issue(Parameters(input())).await;
    assert_eq!(
        serde_json::to_value(&issued).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    let raw = serde_json::to_string(&issued).unwrap();
    assert!(raw.contains("ssh-ed25519-cert-v01"));
    assert!(!raw.contains("PRIVATE KEY"));
    let report = server
        .certificate_check_key(Parameters(CertificateCheckInput {
            item: "Synthetic MCP CA".into(),
            key_module_id: None,
            public_or_certificate: key(71).public_key().to_openssh().unwrap(),
        }))
        .await;
    assert!(!is_error(&report));
    server
        .vault
        .change_signing_permissions(SigningPermissionChange::Enable(false))
        .unwrap();
    assert!(is_error(
        &server.certificate_issue(Parameters(input())).await
    ));
    assert!(
        server
            .vault
            .issue_certificate(
                CertificateActor::Owner,
                id,
                Uuid::parse_str(&rid).unwrap(),
                request(),
                60
            )
            .is_ok()
    );
}
#[test]
fn mcp_writes_preserve_authority_and_cannot_register_or_change_its_visibility() {
    let (_d, server, id) = setup();
    server
        .vault
        .change_write_permissions(WritePermissionChange::AddFolder("agent-test"))
        .unwrap();
    server
        .vault
        .change_write_permissions(WritePermissionChange::Enable(true))
        .unwrap();
    let original = server.vault.get_connection(id).unwrap().stored;
    let update=serde_json::from_value(json!({"id":id,"expectedRevision":original.revision,"itemType":"custom","name":"renamed CA","modules":[]})).unwrap();
    server.vault.agent_update(update).unwrap();
    assert_eq!(
        server
            .vault
            .get_connection(id)
            .unwrap()
            .stored
            .certificate_authority,
        original.certificate_authority
    );
    assert!(
        server
            .resolve_item_for_action("renamed CA", Some("fill"))
            .is_err()
    );
    assert!(
        serde_json::from_value::<crate::vault_write::UpdateInput>(
            json!({"id":id,"expectedRevision":1,"certificateAuthority":null,"modules":[]})
        )
        .is_err()
    );
    assert!(
        serde_json::from_value::<crate::vault_write::ModulePatch>(
            json!({"id":original.modules[0].id,"agentVisible":true})
        )
        .is_err()
    );
    assert!(serde_json::from_value::<CertificateIssueInput>(json!({"item":"renamed CA","requestId":Uuid::new_v4(),"request":request(),"ttlSeconds":60,"actor":"owner"})).is_err());
}
#[test]
fn tool_catalog_describes_certificate_trust_and_retry_boundaries() {
    let (_d, server, _) = setup();
    let names: Vec<_> = server
        .tool_router
        .list_all()
        .iter()
        .map(|tool| tool.name.to_string())
        .collect();
    for name in [
        "certificate_capabilities",
        "certificate_inspect",
        "certificate_verify",
        "certificate_check_key",
        "certificate_issue",
    ] {
        assert!(names.contains(&name.into()));
    }
    let instructions = server.get_info().instructions.unwrap();
    for term in ["CA-marked", "requestId", "ssh-trust set-ca", "revocation"] {
        assert!(instructions.contains(term));
    }
}

#[test]
fn protected_authority_cannot_be_used_as_generic_ssh_jump() {
    let (_d, server, id) = setup();
    let mut edit: ConnectionInput = serde_json::from_value(
        serde_json::to_value(server.vault.get_connection(id).unwrap().stored.portable()).unwrap(),
    )
    .unwrap();
    edit.ssh_auth_type = "privateKey".into();
    edit.modules.extend([
        crate::model::ItemModule {
            kind: "host".into(),
            value: "127.0.0.1".into(),
            ..Default::default()
        },
        crate::model::ItemModule {
            kind: "port".into(),
            value: "22".into(),
            ..Default::default()
        },
        crate::model::ItemModule {
            kind: "username".into(),
            ..Default::default()
        },
    ]);
    edit.secrets
        .named_secrets
        .insert("username".into(), "test-user".into());
    server.vault.save_connection(edit).unwrap();
    let target:ConnectionInput=serde_json::from_value(json!({"name":"Synthetic jump target","itemType":"ssh","sshAuthType":"password","sshJumpItemId":id,"modules":[{"kind":"host","value":"127.0.0.2"},{"kind":"port","value":"22"},{"kind":"username"},{"kind":"password"}],"secrets":{"password":"simulated-target-password","namedSecrets":{"username":"target-user"}}})).unwrap();
    server.vault.save_connection(target).unwrap();
    assert!(
        server
            .prepare_ssh_connection("Synthetic jump target", None, None, None)
            .is_err()
    );
}
