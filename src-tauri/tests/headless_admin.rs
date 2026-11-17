use serde_json::Value;
use std::process::{Command, Output, Stdio};

fn command(data: &tempfile::TempDir, args: &[&str]) -> Command {
    let mut child = Command::new(env!("CARGO_BIN_EXE_openkeylet"));
    child.env("OPENKEYLET_DATA_DIR", data.path()).args(args).stdin(Stdio::null());
    child
}

fn run(data: &tempfile::TempDir, args: &[&str]) -> Output {
    command(data, args).output().unwrap()
}

fn json(output: Output) -> Value {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn permissions_management_uses_one_shared_vault() {
    let data = tempfile::tempdir().unwrap();
    assert_eq!(json(run(&data, &["permissions", "status"]))["effective"], false);
    assert!(!run(&data, &["permissions", "enable"]).status.success());
    let mut a = command(&data, &["permissions", "folder", "add", " agent "])
        .stdout(Stdio::null()).spawn().unwrap();
    let mut b = command(&data, &["permissions", "folder", "add", "other"])
        .stdout(Stdio::null()).spawn().unwrap();
    assert!(a.wait().unwrap().success());
    assert!(b.wait().unwrap().success());
    let status = json(run(&data, &["permissions", "enable"]));
    assert_eq!(status["effective"], true);
    let folders = status["folders"].as_array().unwrap();
    assert_eq!(folders.len(), 2);
    assert!(folders.contains(&Value::from("agent")) && folders.contains(&Value::from("other")));
    assert!(!run(&data, &["permissions", "folder", "add", " agent "]).status.success());
    assert!(!run(&data, &["permissions", "folder", "add", " "]).status.success());
    assert!(run(&data, &["permissions", "folder", "remove", "other"]).status.success());
    assert_eq!(json(run(&data, &["permissions", "folder", "remove", "agent"]))["effective"], false);
    assert!(run(&data, &["permissions", "folder", "add", "agent"]).status.success());
    let disabled = json(run(&data, &["permissions", "disable"]));
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["folders"], serde_json::json!(["agent"]));
}
