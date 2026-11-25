//! Isolated backend for scripts/test-browser.mjs; never opens the user's vault.
use mcp_vault::{
    browser::BrowserBridge,
    model::{ConnectionInput, SettingsPatch},
    vault::Vault,
};
use serde_json::json;
use std::{
    io::Write,
    path::{Path, PathBuf},
};

fn write_onepassword_fixture(path: &Path, website: &str) -> anyhow::Result<()> {
    let mut archive = zip::ZipWriter::new(std::fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    archive.start_file("export.attributes", options)?;
    archive.write_all(
        br#"{"version":3,"description":"1Password Unencrypted Export","createdAt":1710000000}"#,
    )?;
    archive.start_file("export.data", options)?;
    serde_json::to_writer(
        &mut archive,
        &json!({
            "accounts": [{"attrs": {"accountName": "Synthetic QA"}, "vaults": [{
                "attrs": {"name": "Test vault", "uuid": "fixture-vault"},
                "items": [{
                    "uuid": "fixture-item", "state": "active", "categoryUuid": "001", "favIndex": 1,
                    "overview": {"title": "Fixture login", "urls": [{"label": "website", "url": format!("{website}/")}]},
                    "details": {"loginFields": [
                        {"designation": "username", "fieldType": "T", "name": "user", "value": "alice"},
                        {"designation": "password", "fieldType": "P", "name": "password", "value": "fixture-initial-password"}
                    ]}
                }]
            }]}]
        }),
    )?;
    archive.finish()?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let root = PathBuf::from(args.get(1).expect("temporary fixture directory"));
    anyhow::ensure!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("openkeylet-browser-test-"),
        "Expected a test directory"
    );
    let port: u16 = args.get(2).expect("bridge port").parse()?;
    let website = args.get(3).expect("test origin");
    let vault = Vault::open(root.join("vault"))?;
    vault.set_owner_pin("123456")?;
    let keepass = root.join("keepass.xml");
    let source = if args.get(4).is_some_and(|format| format == "1pux") {
        let path = root.join("fixture.1pux");
        write_onepassword_fixture(&path, website)?;
        Some((path, "1pux"))
    } else if keepass.is_file() {
        Some((keepass, "keepass"))
    } else {
        None
    };
    if let Some((path, format)) = source {
        let parsed = mcp_vault::migration::read(&path, format)?;
        let summary = vault.merge_connections(parsed.entries)?;
        anyhow::ensure!(summary.added == 1, "Expected one imported login");
    } else {
        vault.save_browser_login(&json!({"origin":website,"name":"Fixture login","username":"alice","password":"fixture-initial-password"}))?;
    }
    vault.update_settings_patch(SettingsPatch {
        browser_enabled: Some(true),
        browser_port: Some(port),
        ..Default::default()
    })?;
    let bridge = BrowserBridge::new(vault.clone());
    bridge.sync().await;
    bridge.start_quick_pairing().await?;
    println!("READY");
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let control = root.join("desktop-edit.json");
        if control.exists() {
            let input: ConnectionInput = serde_json::from_slice(&std::fs::read(&control)?)?;
            vault.save_connection(input)?;
            std::fs::remove_file(control)?;
            std::fs::write(root.join("desktop-edited"), b"ok")?;
        }
        if root.join("stop").exists() {
            break;
        }
    }
    Ok(())
}
