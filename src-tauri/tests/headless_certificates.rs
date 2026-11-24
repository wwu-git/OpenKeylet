use mcp_vault::{
    model::ConnectionInput,
    vault::{CertificateActor, Vault},
};
use serde_json::{Value, json};
use std::{
    fs,
    process::{Command, Output, Stdio},
};
fn run(dir: &tempfile::TempDir, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openkeylet"))
        .env("OPENKEYLET_DATA_DIR", dir.path())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
fn result(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn owner_cli_registers_grants_issues_and_revokes_using_persistent_vault() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let key = russh::keys::PrivateKey::from(
        russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[91; 32]),
    );
    let input:ConnectionInput=serde_json::from_value(json!({"name":"CLI synthetic CA","itemType":"custom","modules":[{"kind":"privateKey"}],"secrets":{"privateKey":key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().to_string()}})).unwrap();
    let item = vault.save_connection(input).unwrap();
    let fingerprint = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    result(run(
        &dir,
        &[
            "ssh-trust",
            "set-ca",
            &item.id.to_string(),
            "127.0.0.1",
            "22",
            fingerprint,
        ],
    ));
    assert_eq!(
        result(run(&dir, &["ssh-trust", "list"]))[0]["kind"],
        "certificateAuthority"
    );
    let id = item.id.to_string();
    let module = item.modules[0].id.to_string();
    assert_eq!(
        result(run(&dir, &["certificates", "status"]))["enabled"],
        false
    );
    assert!(
        result(run(
            &dir,
            &["certificates", "authority", "set", &id, "ssh", &module]
        ))["authorities"][0]["configured"]
            .as_bool()
            .unwrap()
    );
    let allowed = result(run(
        &dir,
        &[
            "certificates",
            "permissions",
            "allow",
            &id,
            "sshUser",
            "120",
        ],
    ));
    assert_eq!(allowed["grants"][0]["maxTtlSeconds"], 120);
    result(run(&dir, &["certificates", "permissions", "enable"]));
    let public_file = dir.path().join("public.pub");
    fs::write(&public_file, key.public_key().to_openssh().unwrap()).unwrap();
    assert_eq!(
        result(run(
            &dir,
            &[
                "certificates",
                "check-key",
                &id,
                public_file.to_str().unwrap()
            ]
        ))["matches"],
        true
    );
    let request = mcp_vault::certificates::CertificateRequest::SshUser {
        public_key: key.public_key().to_openssh().unwrap(),
        principals: vec!["test-user".into()],
        key_id: "cli".into(),
        extensions: vec![],
    };
    let request_file = dir.path().join("request.json");
    fs::write(&request_file, serde_json::to_vec(&request).unwrap()).unwrap();
    let rid = uuid::Uuid::new_v4();
    let request_id = rid.to_string();
    let issued = result(run(
        &dir,
        &[
            "certificates",
            "issue",
            &id,
            &request_id,
            request_file.to_str().unwrap(),
            "60",
        ],
    ));
    assert_eq!(issued["kind"], "sshUser");
    assert_eq!(
        issued,
        result(run(
            &dir,
            &[
                "certificates",
                "issue",
                &id,
                &request_id,
                request_file.to_str().unwrap(),
                "60"
            ]
        ))
    );
    let cert_file = dir.path().join("issued-cert.pub");
    fs::write(&cert_file, issued["certificate"].as_str().unwrap()).unwrap();
    assert_eq!(
        result(run(
            &dir,
            &["certificates", "inspect", cert_file.to_str().unwrap()]
        ))["kind"],
        "sshUser"
    );
    result(run(&dir, &["certificates", "permissions", "disable"]));
    assert!(
        vault
            .issue_certificate(CertificateActor::Agent, item.id, rid, request, 60)
            .is_err()
    );
    result(run(&dir, &["certificates", "authority", "clear", &id]));
    assert!(vault.certificate_capabilities().unwrap().grants.is_empty());
    assert!(vault.agent_connection(item.id).is_ok());
}

struct McpProcess {
    child: std::process::Child,
    input: std::process::ChildStdin,
    lines: std::sync::mpsc::Receiver<String>,
    next_id: u64,
}
impl McpProcess {
    fn start(dir: &tempfile::TempDir) -> Self {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new(env!("CARGO_BIN_EXE_openkeylet"))
            .env("OPENKEYLET_DATA_DIR", dir.path())
            .args(["mcp", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if sender.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            lines,
            next_id: 0,
        }
    }
    fn send(&mut self, message: Value) {
        use std::io::Write;
        writeln!(self.input, "{message}").unwrap();
        self.input.flush().unwrap();
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let line = self
                .lines
                .recv_timeout(std::time::Duration::from_secs(15))
                .expect("MCP response timed out");
            let response: Value =
                serde_json::from_str(&line).expect("MCP stdout must contain JSON protocol only");
            if response["id"] == id {
                assert!(response.get("error").is_none(), "MCP JSON-RPC error");
                return response["result"].clone();
            }
        }
    }
    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":arguments}))
    }
}
impl Drop for McpProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn data(result: Value) -> Value {
    assert_ne!(
        result["isError"], true,
        "MCP tool unexpectedly rejected the request"
    );
    result["structuredContent"].clone()
}

#[test]
fn stdio_mcp_discovers_and_uses_certificate_tools_with_live_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let vault = Vault::open(dir.path().into()).unwrap();
    let key = russh::keys::PrivateKey::from(
        russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&[92; 32]),
    );
    let input:ConnectionInput=serde_json::from_value(json!({"name":"stdio synthetic CA","itemType":"custom","modules":[{"kind":"privateKey","agentVisible":true}],"secrets":{"privateKey":key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().to_string()}})).unwrap();
    let item = vault.save_connection(input).unwrap();
    let id = item.id.to_string();
    let module = item.modules[0].id.to_string();
    result(run(
        &dir,
        &["certificates", "authority", "set", &id, "ssh", &module],
    ));
    let mut mcp = McpProcess::start(&dir);
    let initialized=mcp.rpc("initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"certificate-acceptance","version":"1"}}));
    let instructions = initialized["instructions"].as_str().unwrap();
    for term in [
        "certificate_issue",
        "CA-marked",
        "requestId",
        "ssh-trust set-ca",
    ] {
        assert!(instructions.contains(term));
    }
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let tools = mcp.rpc("tools/list", json!({}));
    for name in [
        "certificate_capabilities",
        "certificate_inspect",
        "certificate_verify",
        "certificate_check_key",
        "certificate_issue",
    ] {
        let tool = tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap();
        assert!(tool["outputSchema"].is_object());
        assert!(tool["annotations"].is_object());
    }
    let issue_schema = tools["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["name"] == "certificate_issue")
        .unwrap()["inputSchema"]
        .to_string();
    for field in [
        "sshUser",
        "sshHost",
        "tlsServer",
        "tlsClient",
        "principals",
        "csr",
        "ttlSeconds",
        "requestId",
    ] {
        assert!(issue_schema.contains(field));
    }
    assert!(!issue_schema.contains("\"actor\""));
    assert_eq!(
        data(mcp.tool("certificate_capabilities", json!({})))["enabled"],
        false
    );
    let request_id = uuid::Uuid::new_v4();
    let arguments = json!({"item":"stdio synthetic CA","requestId":request_id,"ttlSeconds":60,"request":{"kind":"sshUser","publicKey":key.public_key().to_openssh().unwrap(),"principals":["test-user"],"keyId":"stdio","extensions":[]}});
    assert_eq!(
        mcp.tool("certificate_issue", arguments.clone())["isError"],
        true
    );
    result(run(
        &dir,
        &[
            "certificates",
            "permissions",
            "allow",
            &id,
            "sshUser",
            "120",
        ],
    ));
    result(run(&dir, &["certificates", "permissions", "enable"]));
    assert_eq!(
        data(mcp.tool("certificate_capabilities", json!({})))["enabled"],
        true
    );
    let issued = data(mcp.tool("certificate_issue", arguments.clone()));
    assert_eq!(issued["kind"], "sshUser");
    assert_eq!(
        issued,
        data(mcp.tool("certificate_issue", arguments.clone()))
    );
    let inspected = data(mcp.tool(
        "certificate_inspect",
        json!({"certificate":issued["certificate"]}),
    ));
    assert_eq!(inspected["serial"], issued["serial"]);
    assert_eq!(
        data(mcp.tool(
            "certificate_check_key",
            json!({"item":"stdio synthetic CA","publicOrCertificate":issued["certificate"]})
        ))["matches"],
        true
    );
    let verified=data(mcp.tool("certificate_verify",json!({"certificate":issued["certificate"],"authorities":[key.public_key().to_openssh().unwrap()],"intermediates":[],"identity":"test-user","kind":"sshUser"})));
    assert_eq!(verified["verified"], true);
    assert_eq!(verified["revocationChecked"], false);
    let searched = data(mcp.tool("items_search", json!({"query":"stdio synthetic CA"})));
    assert_eq!(searched["items"][0]["modules"][0]["agentVisible"], false);
    assert!(searched["items"][0]["modules"][0]["value"].is_null());
    assert!(!searched.to_string().contains("PRIVATE KEY"));
    result(run(&dir, &["certificates", "permissions", "remove", &id]));
    assert_eq!(mcp.tool("certificate_issue", arguments)["isError"], true);
    result(run(&dir, &["certificates", "permissions", "disable"]));
    assert_eq!(
        data(mcp.tool("certificate_capabilities", json!({})))["enabled"],
        false
    );
}
