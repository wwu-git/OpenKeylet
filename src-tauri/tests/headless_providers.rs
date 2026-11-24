use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{Receiver, channel},
    time::Duration,
};

struct Mcp {
    child: Child,
    input: ChildStdin,
    lines: Receiver<String>,
    id: u64,
}
impl Mcp {
    fn start(path: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_openkeylet"))
            .env("OPENKEYLET_DATA_DIR", path)
            .args(["mcp", "stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if send.send(line.unwrap()).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input,
            lines,
            id: 0,
        }
    }
    fn send(&mut self, value: Value) {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
    }
    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let id = self.id;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}));
        loop {
            let response: Value =
                serde_json::from_str(&self.lines.recv_timeout(Duration::from_secs(15)).unwrap())
                    .unwrap();
            if response["id"] == id {
                assert!(response.get("error").is_none(), "MCP protocol failure");
                return response["result"].clone();
            }
        }
    }
    fn tool(&mut self, name: &str, args: Value) -> Value {
        self.rpc("tools/call", json!({"name":name,"arguments":args}))
    }
}
impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn data(value: Value) -> Value {
    assert_ne!(value["isError"], true, "MCP tool failed");
    value["structuredContent"].clone()
}

#[test]
fn stdio_provider_discovery_bindings_hidden_values_and_live_write_revoke() {
    let dir = tempfile::tempdir().unwrap();
    let mut mcp = Mcp::start(dir.path());
    let init=mcp.rpc("initialize",json!({"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"provider-acceptance","version":"1"}}));
    for term in [
        "api_service_capabilities",
        "aws-sigv4",
        "awsBindings",
        "pagination",
    ] {
        assert!(init["instructions"].as_str().unwrap().contains(term));
    }
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    let tools = mcp.rpc("tools/list", json!({}));
    let tools = tools["tools"].as_array().unwrap();
    let service = tools
        .iter()
        .find(|tool| tool["name"] == "api_service_capabilities")
        .unwrap();
    assert!(service["outputSchema"].is_object());
    for name in ["items_create", "items_update"] {
        assert!(
            tools.iter().find(|tool| tool["name"] == name).unwrap()["inputSchema"]
                .to_string()
                .contains("awsBindings")
        );
    }
    let identified = data(mcp.tool(
        "api_service_capabilities",
        json!({"url":"https://ec2.us-east-1.amazonaws.com/"}),
    ));
    assert_eq!(identified["provider"], "aws");
    assert_eq!(identified["authentication"], "aws-sigv4");
    assert_eq!(identified["status"], "needsConfiguration");
    for url in [
        "http://ec2.us-east-1.amazonaws.com/",
        "https://user@ec2.us-east-1.amazonaws.com/",
    ] {
        assert_eq!(
            mcp.tool("api_service_capabilities", json!({"url":url}))["isError"],
            true
        );
    }
    assert_eq!(
        mcp.tool(
            "api_service_capabilities",
            json!({"item":"unused","url":"https://api.github.com/"})
        )["isError"],
        true
    );
    assert_eq!(
        data(mcp.tool(
            "api_service_capabilities",
            json!({"url":"https://s3.us-east-1.amazonaws.com/"})
        ))["status"],
        "unsupported"
    );
    let payload = json!({"name":"Synthetic provider API","itemType":"api","folder":"agent-test","httpAuthType":"aws-sigv4","modules":[{"kind":"url","value":"https://ec2.us-east-1.amazonaws.com/"},{"kind":"customSecret","name":"region","value":"us-east-1"},{"kind":"customSecret","name":"access id","value":"AKIDSYNTHETIC"},{"kind":"customSecret","name":"secret key","value":"synthetic-aws-secret"}]});
    assert_eq!(mcp.tool("items_create", payload.clone())["isError"], true);
    let cli = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_openkeylet"))
            .env("OPENKEYLET_DATA_DIR", dir.path())
            .args(args)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(output.status.success());
    };
    cli(&["permissions", "folder", "add", "agent-test"]);
    cli(&["permissions", "enable"]);
    let created = data(mcp.tool("items_create", payload));
    let id = created["id"].as_str().unwrap();
    let modules = created["moduleIds"].as_array().unwrap();
    let bound=data(mcp.tool("items_update",json!({"id":id,"expectedRevision":created["revision"],"awsBindings":{"serviceUrl":modules[0],"region":modules[1],"accessKeyId":modules[2],"secretAccessKey":modules[3]}})));
    let ready = data(mcp.tool(
        "api_service_capabilities",
        json!({"item":"Synthetic provider API"}),
    ));
    assert_eq!(ready["status"], "ready");
    assert!(
        ready.get("region").is_none(),
        "Hidden region metadata must not be disclosed"
    );
    let found = data(mcp.tool("items_search", json!({"query":"Synthetic provider API"})));
    assert_eq!(found["items"][0]["apiService"]["status"], "ready");
    let raw = found.to_string();
    assert!(!raw.contains("synthetic-aws-secret"));
    assert!(!raw.contains("AKIDSYNTHETIC"));
    assert!(found["items"][0]["awsBindings"]["secretAccessKey"].is_string());
    assert_eq!(
        mcp.tool(
            "items_update",
            json!({"id":id,"expectedRevision":created["revision"],"description":"stale"})
        )["isError"],
        true
    );
    cli(&["permissions", "disable"]);
    assert_eq!(
        mcp.tool(
            "items_update",
            json!({"id":id,"expectedRevision":bound["revision"],"description":"revoked"})
        )["isError"],
        true
    );
}
