use anyhow::{Result, bail};
use mcp_vault::vault::{Vault, WritePermissionChange};
use serde::Serialize;

fn print_json(data: impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

pub(crate) fn permissions(vault: &Vault, args: &[String]) -> Result<()> {
    use WritePermissionChange::{AddFolder, Enable, RemoveFolder};
    let change = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status"] => None,
        ["enable"] => Some(Enable(true)),
        ["disable"] => Some(Enable(false)),
        ["folder", "add", name] => Some(AddFolder(name)),
        ["folder", "remove", name] => Some(RemoveFolder(name)),
        _ => bail!("Usage: openkeylet permissions <status|enable|disable|folder add/remove NAME>"),
    };
    let settings = match change {
        Some(change) => vault.change_write_permissions(change)?,
        None => vault.settings()?,
    };
    print_json(serde_json::json!({
        "enabled": settings.agent_write_enabled,
        "effective": settings.mcp_writes_enabled(),
        "folders": settings.agent_write_folders,
    }))
}
