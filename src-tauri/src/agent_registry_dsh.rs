use super::{EntryState, backup_config, classify_command, write_atomic_checked};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::HashSet,
    fs,
    io::{ErrorKind, Read},
    path::{Component, Path, PathBuf},
};
use yaml_edit::{Mapping, Scalar, Sequence, YamlFile, YamlNode};

const MAX_PATCH_BYTES: u64 = 1024 * 1024;
const PATCH_FILE: &str = "cordis.patch.yml";
const ENTRY_ID: &str = "mcp-openkeylet";
const LEGACY_ID: &str = "mcp-kru";
const PLUGIN: &str = "@deepseek-ai/dsh-mcp-client";

pub(super) fn config_path(home: &Path, explicit_profile: Option<&str>) -> Result<Option<PathBuf>> {
    let profiles = home.join("profiles");
    if let Some(name) = explicit_profile {
        ensure!(
            !name.contains(['/', '\\'])
                && matches!(
                    Path::new(name).components().collect::<Vec<_>>().as_slice(),
                    [Component::Normal(_)]
                ),
            "DSH_PROFILE must be a single profile name"
        );
        let profile = profiles.join(name);
        ensure!(
            profile.is_dir(),
            "The configured DeepSeek Harness profile does not exist"
        );
        return Ok(Some(profile.join(PATCH_FILE)));
    }

    let entries = match fs::read_dir(&profiles) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("Unable to list DeepSeek Harness profiles"),
    };
    let mut selected = None;
    for entry in entries {
        let entry = entry.context("Unable to read a DeepSeek Harness profile")?;
        if !entry.file_type()?.is_dir() || entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        ensure!(
            selected.is_none(),
            "Multiple DeepSeek Harness profiles found; set DSH_PROFILE to choose one"
        );
        selected = Some(entry.path().join(PATCH_FILE));
    }
    Ok(selected)
}

#[derive(Clone, Copy)]
enum EntryLocation {
    Direct { index: usize },
    Inserted { patch: usize, index: usize },
}

enum Entry {
    Missing,
    Owned {
        location: EntryLocation,
        state: EntryState,
    },
    Conflict,
}

impl Entry {
    fn state(self) -> EntryState {
        match self {
            Self::Missing => EntryState::Available,
            Self::Owned { state, .. } => state,
            Self::Conflict => EntryState::Conflict,
        }
    }
}

fn parse_patch(text: &str) -> Result<(YamlFile, Sequence)> {
    let yaml = text.parse::<YamlFile>().map_err(|_| {
        anyhow::anyhow!("Invalid DeepSeek Harness YAML patch; no changes were made")
    })?;
    let mut documents = yaml.documents();
    let rows = documents
        .next()
        .and_then(|document| document.as_sequence())
        .context("DeepSeek Harness patch must contain one YAML sequence")?;
    ensure!(
        documents.next().is_none(),
        "DeepSeek Harness patch must contain one YAML document"
    );
    drop(documents);
    Ok((yaml, rows))
}

fn scalar(node: &YamlNode) -> Option<String> {
    node.as_scalar().map(Scalar::as_string)
}

fn field(mapping: &Mapping, key: &str) -> Option<String> {
    mapping.get(key).as_ref().and_then(scalar)
}

fn check_keys(mapping: &Mapping) -> Result<()> {
    let mut keys = HashSet::new();
    for (key, _) in mapping.iter() {
        let key = scalar(&key).context("Unsupported key in DeepSeek Harness patch")?;
        ensure!(
            key != "<<" && keys.insert(key),
            "Ambiguous keys in DeepSeek Harness patch; no changes were made"
        );
    }
    Ok(())
}

fn mapping_at(rows: &Sequence, index: usize) -> Result<Mapping> {
    rows.get(index)
        .and_then(|node| node.as_mapping().cloned())
        .context("DeepSeek Harness patch entries must be YAML mappings")
}

fn classify_entry(row: &Mapping, executable: &Path) -> Result<Option<EntryState>> {
    check_keys(row)?;
    let id = field(row, "id");
    if row.get("id").is_some() && id.is_none() {
        bail!("Unsupported entry ID in DeepSeek Harness patch")
    }
    let plugin = field(row, "name");
    let config_node = row.get("config");
    let config = config_node.as_ref().and_then(YamlNode::as_mapping);
    if plugin.as_deref() == Some(PLUGIN) && config_node.is_some() && config.is_none() {
        bail!("Unsupported MCP configuration in DeepSeek Harness patch");
    }
    let server = if let Some(config) = config {
        check_keys(config)?;
        let server = field(config, "serverName");
        if plugin.as_deref() == Some(PLUGIN)
            && config.get("serverName").is_some()
            && server.is_none()
        {
            bail!("Unsupported MCP server name in DeepSeek Harness patch");
        }
        server
    } else {
        None
    };
    let identity = id.as_deref() == Some(ENTRY_ID) && server.as_deref() == Some("openkeylet");
    let legacy = id.as_deref() == Some(LEGACY_ID) && server.as_deref() == Some("kru");
    if id.as_deref() != Some(ENTRY_ID) && server.as_deref() != Some("openkeylet") && !legacy {
        return Ok(None);
    }

    let unknown = if legacy {
        None
    } else {
        Some(EntryState::Conflict)
    };
    let (Some(config), true) = (config, identity || legacy) else {
        return Ok(unknown);
    };
    if plugin.as_deref() != Some(PLUGIN) || field(config, "transport").as_deref() != Some("stdio") {
        return Ok(unknown);
    }
    let Some(command) = field(config, "command") else {
        return Ok(unknown);
    };
    let args = config
        .get("args")
        .and_then(|value| {
            let items = value.as_sequence()?;
            (0..items.len())
                .map(|index| items.get(index).as_ref().and_then(scalar))
                .collect::<Option<Vec<_>>>()
        })
        .unwrap_or_default();
    let current = match classify_command(&command, &args, executable) {
        EntryState::Registered => identity,
        EntryState::Stale => false,
        _ => return Ok(unknown),
    };
    let disabled = match row.get("disabled") {
        None => false,
        Some(node) => match node.as_scalar() {
            Some(value) if value.is_null() => false,
            Some(value) => match value.as_bool().filter(|_| !value.is_quoted()) {
                Some(value) => value,
                None => return Ok(Some(EntryState::Conflict)),
            },
            None => return Ok(Some(EntryState::Conflict)),
        },
    };
    Ok(Some(if disabled {
        EntryState::Disabled
    } else if current {
        EntryState::Registered
    } else {
        EntryState::Stale
    }))
}

fn find_entry(rows: &Sequence, executable: &Path) -> Result<Entry> {
    let mut found = Entry::Missing;
    let mut record = |location, state| {
        found = match found {
            Entry::Missing if state != EntryState::Conflict => Entry::Owned { location, state },
            _ => Entry::Conflict,
        };
    };
    for patch in 0..rows.len() {
        let row = mapping_at(rows, patch)?;
        let direct = classify_entry(&row, executable)?;
        if let Some(state) = direct {
            let state = if state == EntryState::Registered {
                EntryState::Stale
            } else {
                state
            };
            record(EntryLocation::Direct { index: patch }, state);
        }
        if let Some(insert) = row.get("insert") {
            let children = insert
                .as_sequence()
                .context("Unsupported DeepSeek Harness insert patch")?;
            for index in 0..children.len() {
                let child = mapping_at(children, index)?;
                if let Some(state) = classify_entry(&child, executable)? {
                    let state = if row.get("id").is_some() {
                        EntryState::Conflict
                    } else {
                        state
                    };
                    record(EntryLocation::Inserted { patch, index }, state);
                }
            }
        }
    }
    Ok(found)
}

fn home_patch_conflicts(path: &Path, executable: &Path) -> Result<bool> {
    let Some(profiles) = path.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    if profiles.file_name().is_none_or(|name| name != "profiles") {
        return Ok(false);
    }
    let Some(home) = profiles.parent() else {
        return Ok(false);
    };
    let (_, _, rows) = load_patch(&home.join(PATCH_FILE))?;
    Ok(!matches!(find_entry(&rows, executable)?, Entry::Missing))
}

fn read_patch(path: &Path) -> Result<String> {
    super::reject_linked_config(path)?;
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(String::new()),
        Err(error) => return Err(error).context("Unable to inspect DeepSeek Harness patch"),
    };
    ensure!(
        metadata.is_file(),
        "DeepSeek Harness patch must be a regular file"
    );
    ensure!(
        metadata.len() <= MAX_PATCH_BYTES,
        "DeepSeek Harness patch exceeds 1 MiB"
    );
    let file = fs::File::open(path).context("Unable to open DeepSeek Harness patch")?;
    let mut text = String::new();
    file.take(MAX_PATCH_BYTES + 1)
        .read_to_string(&mut text)
        .context("Unable to read DeepSeek Harness patch")?;
    ensure!(
        text.len() as u64 <= MAX_PATCH_BYTES,
        "DeepSeek Harness patch exceeds 1 MiB"
    );
    Ok(text)
}

fn load_patch(path: &Path) -> Result<(String, YamlFile, Sequence)> {
    let original = read_patch(path)?;
    let initial;
    let text = if original
        .lines()
        .all(|line| line.trim().is_empty() || line.trim_start().starts_with('#'))
    {
        let separator = if original.is_empty() || original.ends_with('\n') {
            ""
        } else {
            "\n"
        };
        initial = format!("{original}{separator}[]\n");
        &initial
    } else {
        &original
    };
    let (yaml, rows) = parse_patch(text)?;
    Ok((original, yaml, rows))
}

pub(super) fn inspect(path: &Path, executable: &Path) -> Result<EntryState> {
    if home_patch_conflicts(path, executable)? {
        return Ok(EntryState::Conflict);
    }
    let (_, _, rows) = load_patch(path)?;
    Ok(find_entry(&rows, executable)?.state())
}

fn insert_entry(rows: &Sequence, row: Mapping) {
    let children = Sequence::new_flow();
    children.push(row);
    let patch = Mapping::new_flow();
    patch.set("insert", children);
    rows.push(patch);
}

fn inserted_entries(rows: &Sequence, patch: usize) -> Result<Sequence> {
    mapping_at(rows, patch)?
        .get("insert")
        .and_then(|value| value.as_sequence().cloned())
        .context("Missing DeepSeek Harness insert patch")
}

fn configure(row: &Mapping, command: &str) -> Result<()> {
    let config = row
        .get("config")
        .and_then(|value| value.as_mapping().cloned())
        .context("Missing DeepSeek Harness MCP config")?;
    let args = Sequence::new_flow();
    args.push("mcp");
    args.push("stdio");
    row.set("id", ENTRY_ID);
    if row.get("disabled").is_some() {
        row.set("disabled", false);
    }
    config.set("serverName", "openkeylet");
    config.set("transport", "stdio");
    config.set("command", command);
    config.set("args", args);
    Ok(())
}

pub(super) fn mutate(path: &Path, executable: &Path, action: &str) -> Result<()> {
    ensure!(
        !home_patch_conflicts(path, executable)?,
        "The DeepSeek Harness home patch already configures OpenKeylet"
    );
    let (original, yaml, rows) = load_patch(path)?;
    let entry = find_entry(&rows, executable)?;
    let command = executable
        .to_str()
        .context("OpenKeylet executable path is not valid UTF-8")?;
    match (action, entry) {
        (_, Entry::Conflict) => {
            bail!("A conflicting DeepSeek Harness entry exists; no changes were made")
        }
        ("register", Entry::Missing) => {
            let row = Mapping::new_flow();
            row.set("name", PLUGIN);
            row.set("config", Mapping::new_flow());
            configure(&row, command)?;
            insert_entry(&rows, row);
        }
        (
            "repair",
            Entry::Owned {
                location: EntryLocation::Direct { index },
                ..
            },
        ) => {
            let row = mapping_at(&rows, index)?;
            configure(&row, command)?;
            rows.remove(index)
                .context("Missing DeepSeek Harness patch entry")?;
            insert_entry(&rows, row);
        }
        (
            "repair",
            Entry::Owned {
                location: EntryLocation::Inserted { patch, index },
                ..
            },
        ) => {
            let siblings = inserted_entries(&rows, patch)?;
            configure(&mapping_at(&siblings, index)?, command)?;
        }
        (
            "remove",
            Entry::Owned {
                location: EntryLocation::Direct { index },
                ..
            },
        ) => {
            rows.remove(index)
                .context("Missing DeepSeek Harness patch entry")?;
        }
        (
            "remove",
            Entry::Owned {
                location: EntryLocation::Inserted { patch, index },
                ..
            },
        ) => {
            let siblings = inserted_entries(&rows, patch)?;
            if siblings.len() == 1 && mapping_at(&rows, patch)?.len() == 1 {
                rows.remove(patch)
                    .context("Missing DeepSeek Harness insert patch")?;
            } else {
                siblings
                    .remove(index)
                    .context("Missing DeepSeek Harness MCP entry")?;
            }
        }
        _ => bail!("Cannot perform this DeepSeek Harness operation in the current state"),
    }
    let updated = yaml.to_string();
    let (_, checked) = parse_patch(&updated)?;
    let expected = if action == "remove" {
        EntryState::Available
    } else {
        EntryState::Registered
    };
    ensure!(
        find_entry(&checked, executable)?.state() == expected,
        "Unable to verify the updated DeepSeek Harness patch"
    );
    if updated != original {
        ensure!(
            !home_patch_conflicts(path, executable)?,
            "The DeepSeek Harness home patch already configures OpenKeylet"
        );
        if action == "repair" && path.is_file() {
            backup_config(path)?;
        }
        ensure!(
            read_patch(path)? == original,
            "DeepSeek Harness patch changed; rescan and try again"
        );
        write_atomic_checked(path, &original, &updated)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsh_resolves_only_existing_unambiguous_profiles() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        assert!(config_path(home, None).unwrap().is_none());
        assert!(config_path(home, Some("../outside")).is_err());
        assert!(config_path(home, Some("web")).is_err());
        fs::create_dir_all(home.join("profiles/web")).unwrap();
        let web = home.join("profiles/web/cordis.patch.yml");
        assert_eq!(config_path(home, None).unwrap(), Some(web.clone()));
        fs::create_dir_all(home.join("profiles/desktop")).unwrap();
        assert!(config_path(home, None).is_err());
        assert_eq!(config_path(home, Some("web")).unwrap(), Some(web));
    }

    #[test]
    fn dsh_registration_repair_and_removal_preserve_user_patch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PATCH_FILE);
        let user = "# Keep this comment\n- id: notes\n  name: example-notes\n  config: { enabled: true, marker: 'kept' }\n";
        fs::write(&path, user).unwrap();
        let old = directory.path().join("old/OpenKeylet");
        let executable = directory.path().join("OpenKeylet with spaces/openkeylet");
        mutate(&path, &old, "register").unwrap();
        assert_eq!(inspect(&path, &old).unwrap(), EntryState::Registered);
        let registered = fs::read_to_string(&path).unwrap();
        let (_, patches) = parse_patch(&registered).unwrap();
        let inserted = inserted_entries(&patches, 1).unwrap();
        assert_eq!(
            field(&mapping_at(&inserted, 0).unwrap(), "id").as_deref(),
            Some(ENTRY_ID)
        );
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Stale);
        mutate(&path, &executable, "repair").unwrap();
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Registered);
        let updated = fs::read_to_string(&path).unwrap();
        assert!(updated.contains(user.trim_end()));
        mutate(&path, &executable, "remove").unwrap();
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Available);
        assert_eq!(
            fs::read_to_string(&path).unwrap().trim_end(),
            user.trim_end()
        );
    }

    #[test]
    fn dsh_repair_migrates_old_direct_row_to_an_insert_patch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PATCH_FILE);
        let executable = directory.path().join("openkeylet");
        let command = serde_json::to_string(executable.to_str().unwrap()).unwrap();
        fs::write(&path, format!("- id: mcp-openkeylet\n  name: '@deepseek-ai/dsh-mcp-client'\n  config: {{serverName: openkeylet, transport: stdio, command: {command}, args: [mcp, stdio], timeoutMs: 2000}}\n")).unwrap();
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Stale);
        mutate(&path, &executable, "repair").unwrap();
        let updated = fs::read_to_string(&path).unwrap();
        let (_, patches) = parse_patch(&updated).unwrap();
        let entries = inserted_entries(&patches, 0).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Registered);
        assert!(updated.contains("timeoutMs: 2000"));
    }

    #[test]
    fn dsh_refuses_to_write_when_home_patch_overrides_openkeylet() {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path();
        fs::create_dir_all(home.join("profiles/web")).unwrap();
        let path = config_path(home, Some("web")).unwrap().unwrap();
        let executable = home.join("openkeylet");
        let home_patch = home.join(PATCH_FILE);
        let override_text = "- id: mcp-openkeylet\n  disabled: true\n";
        fs::write(&home_patch, override_text).unwrap();
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Conflict);
        assert!(mutate(&path, &executable, "register").is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(home_patch).unwrap(), override_text);
    }

    #[test]
    fn dsh_foreign_entries_and_ambiguous_patches_are_never_replaced() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(PATCH_FILE);
        let executable = directory.path().join("openkeylet");
        let qualified = "- id: mcp-openkeylet\n  name: '@deepseek-ai/dsh-mcp-client'\n  config: {serverName: openkeylet, transport: stdio, command: /other/tool, args: [mcp, stdio]}\n";
        fs::write(&path, qualified).unwrap();
        assert_eq!(inspect(&path, &executable).unwrap(), EntryState::Conflict);
        for action in ["register", "repair", "remove"] {
            assert!(mutate(&path, &executable, action).is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), qualified);
        }
        let docs = format!("{qualified}---\n[]\n");
        fs::write(&path, &docs).unwrap();
        assert!(mutate(&path, &executable, "register").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), docs);
    }
}
