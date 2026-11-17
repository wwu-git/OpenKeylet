use anyhow::{Context, Result, bail, ensure};
use mcp_vault::{
    github_backup::{ConnectInput, GithubBackup, generate_recovery_key},
    vault::{Vault, WritePermissionChange},
};
use serde::Serialize;
use std::{
    fs::{File, OpenOptions},
    io::{self, IsTerminal, Write},
};
use zeroize::Zeroizing;

fn print_json(data: impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&data)?);
    Ok(())
}

pub(crate) fn permissions(vault: &Vault, args: &[String]) -> Result<()> {
    use WritePermissionChange::{AddFolder, Enable, RemoveFolder};
    let change = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
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

const BACKUP_USAGE: &str = "Usage: openkeylet github-backup <status|upload|restore|disconnect|connect OWNER/REPO --mode new|existing --auth gh>";

#[derive(Debug, PartialEq, Eq)]
enum BackupMode {
    New,
    Existing,
}

impl BackupMode {
    fn as_str(&self) -> &'static str {
        match self {
            Self::New => "new",
            Self::Existing => "existing",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum BackupCommand<'a> {
    Status,
    Connect {
        repository: &'a str,
        mode: BackupMode,
    },
    Upload,
    Restore,
    Disconnect,
}

fn parse_backup_command<'a>(args: &[&'a str]) -> Result<BackupCommand<'a>> {
    match args {
        ["status"] => Ok(BackupCommand::Status),
        ["upload"] => Ok(BackupCommand::Upload),
        ["restore"] => Ok(BackupCommand::Restore),
        ["disconnect"] => Ok(BackupCommand::Disconnect),
        ["connect", repository, "--mode", mode, "--auth", "gh"]
        | ["connect", repository, "--auth", "gh", "--mode", mode] => {
            let mode = match *mode {
                "new" => BackupMode::New,
                "existing" => BackupMode::Existing,
                _ => bail!("{BACKUP_USAGE}"),
            };
            Ok(BackupCommand::Connect { repository, mode })
        }
        _ => bail!("{BACKUP_USAGE}"),
    }
}

fn open_admin_terminal() -> Result<File> {
    ensure!(
        io::stdin().is_terminal(),
        "GitHub backup connect requires an interactive terminal"
    );
    let path = if cfg!(windows) { "CONOUT$" } else { "/dev/tty" };
    OpenOptions::new()
        .write(true)
        .open(path)
        .context("GitHub backup connect requires an interactive terminal")
}

fn connect_input(
    repository: &str,
    mode: BackupMode,
    terminal: &mut impl Write,
    read_hidden: impl FnOnce() -> io::Result<String>,
) -> Result<ConnectInput> {
    let recovery_key = match mode {
        BackupMode::Existing => {
            write!(terminal, "Recovery key (input hidden): ")?;
            terminal.flush()?;
            let input = Zeroizing::new(read_hidden().context("Could not read the recovery key")?);
            writeln!(terminal)?;
            ensure!(
                !input.trim().is_empty(),
                "GitHub backup connection cancelled"
            );
            input
        }
        BackupMode::New => {
            let key = Zeroizing::new(generate_recovery_key()?);
            writeln!(terminal, "Save this recovery key in a safe place:")?;
            writeln!(terminal, "{}", key.as_str())?;
            write!(terminal, "Type yes after saving the key: ")?;
            terminal.flush()?;
            let answer = Zeroizing::new(read_hidden().context("Could not read the confirmation")?);
            writeln!(terminal)?;
            ensure!(
                answer.trim().eq_ignore_ascii_case("yes"),
                "GitHub backup connection cancelled"
            );
            key
        }
    };
    Ok(ConnectInput {
        repository: repository.to_owned(),
        token: String::new(),
        recovery_key: recovery_key.trim().to_owned(),
        mode: mode.as_str().to_owned(),
        recovery_key_saved: true,
        auth_source: "cli".into(),
        create_repository: false,
    })
}

fn backup_error(error: anyhow::Error) -> anyhow::Error {
    // Only display the backend's stable error code. Never echo raw HTTP errors.
    let code = error.to_string();
    let hint = match code.as_str() {
        "GITHUB_BACKUP_MISSING" => {
            "Remote snapshot missing. Check the repository, branch, and GitHub history."
        }
        "GITHUB_RESTORE_REQUIRED" | "GITHUB_REMOTE_CHANGED" => {
            "Restore the remote snapshot before uploading."
        }
        "GITHUB_BUSY" => "Another backup operation is running for this vault. Retry shortly.",
        "GITHUB_CLI_MISSING" => "Install GitHub CLI (gh) and sign in.",
        "GITHUB_CLI_AUTH" => "Sign in with gh auth login for github.com.",
        "GITHUB_KEY_MISMATCH" | "GITHUB_KEY_INVALID" => "Check the recovery key.",
        "GITHUB_PRIVATE_REQUIRED" => "Select a private GitHub repository.",
        "GITHUB_UPLOAD_UNCONFIRMED" => {
            "Check the repository and restore before attempting another upload."
        }
        _ => "",
    };
    if !hint.is_empty() {
        anyhow::anyhow!("{code}: {hint}")
    } else if code.starts_with("GITHUB_")
        && code.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')
    {
        anyhow::anyhow!("{code}")
    } else {
        anyhow::anyhow!("GITHUB_OPERATION_FAILED")
    }
}

pub(crate) async fn github_backup(vault: Vault, args: &[String]) -> Result<()> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let command = parse_backup_command(&args)?;
    let backup = GithubBackup::new(vault).map_err(backup_error)?;
    match command {
        BackupCommand::Status => print_json(backup.status().map_err(backup_error)?),
        BackupCommand::Connect { repository, mode } => {
            let mut terminal = open_admin_terminal()?;
            let input = connect_input(repository, mode, &mut terminal, rpassword::read_password)?;
            print_json(backup.connect(input).await.map_err(backup_error)?)
        }
        BackupCommand::Upload => print_json(backup.upload().await.map_err(backup_error)?),
        BackupCommand::Restore => print_json(backup.restore().await.map_err(backup_error)?),
        BackupCommand::Disconnect => print_json(backup.disconnect().map_err(backup_error)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_connect_arguments_and_terminal_flow() {
        let mut terminal = Vec::new();
        let command = parse_backup_command(&[
            "connect",
            "demo/vault",
            "--auth",
            "gh",
            "--mode",
            "existing",
        ])
        .unwrap();
        let BackupCommand::Connect { repository, mode } = command else {
            panic!("expected connect")
        };
        let input = connect_input(
            repository,
            mode,
            &mut terminal,
            || Ok("  fake-key  ".into()),
        )
        .unwrap();
        assert_eq!(input.repository, "demo/vault");
        assert_eq!(input.mode, "existing");
        assert_eq!(input.auth_source, "cli");
        assert!(input.recovery_key_saved && !input.create_repository);
        assert_eq!(input.recovery_key, "fake-key");
        assert!(!String::from_utf8(terminal).unwrap().contains("fake-key"));
        assert!(
            connect_input("demo/vault", BackupMode::Existing, &mut Vec::new(), || Ok(
                String::new()
            ))
            .is_err()
        );

        let mut terminal = Vec::new();
        let input = connect_input("demo/vault", BackupMode::New, &mut terminal, || {
            Ok("yes".into())
        })
        .unwrap();
        assert!(input.recovery_key.starts_with("okv1_"));
        assert_eq!(
            String::from_utf8(terminal)
                .unwrap()
                .matches(input.recovery_key.as_str())
                .count(),
            1
        );
        assert!(
            connect_input("demo/vault", BackupMode::New, &mut Vec::new(), || Ok(
                "no".into()
            ))
            .is_err()
        );
    }
}
