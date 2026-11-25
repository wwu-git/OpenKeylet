use crate::{
    model::{ConnectionInput, ItemModule, SecretBundle},
    vault::{Vault, move_connection_to_trash, same_item_name},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fs::File, io::Read, path::Path};
use uuid::Uuid;
use zeroize::{ZeroizeOnDrop, Zeroizing};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_ENTRIES: usize = 256;
const MAX_VALUE_BYTES: usize = 65_536;
const VALUE_FIELD: &str = "Value";

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
struct Variable {
    name: String,
    value: String,
}

#[derive(Serialize, Deserialize)]
pub(super) struct EnvImport {
    variables: Vec<Variable>,
    empty: usize,
    multiline: usize,
}

#[derive(Serialize)]
pub(super) struct Preview {
    id: Uuid,
    rows: Vec<PreviewRow>,
    empty: usize,
    multiline: usize,
}

#[derive(Serialize)]
struct PreviewRow {
    index: usize,
    name: String,
    conflict: bool,
}

pub(crate) struct UndoBatch {
    pub id: Uuid,
    created: Vec<(Uuid, u64)>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CommitSummary {
    pub added: usize,
    skipped: usize,
    undo_id: Option<Uuid>,
}

#[derive(Serialize)]
pub(super) struct UndoSummary {
    pub removed: usize,
    skipped: usize,
}

impl EnvImport {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = path
            .metadata()
            .context("Could not inspect the selected .env file")?;
        ensure!(metadata.is_file(), "Choose a regular .env file");
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "The .env file exceeds 1 MiB"
        );
        let file = File::open(path).context("Could not open the selected .env file")?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("Could not read the selected .env file")?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "The .env file exceeds 1 MiB"
        );
        Self::parse(std::str::from_utf8(&bytes).context("The .env file must be UTF-8")?)
    }

    fn parse(text: &str) -> Result<Self> {
        ensure!(
            !text.contains('\0'),
            "The .env file contains a NUL character"
        );
        let mut lines = text.trim_start_matches('\u{feff}').lines().enumerate();
        let mut variables = Vec::new();
        let mut empty = 0;
        let mut multiline = 0;
        while let Some((index, line)) = lines.next() {
            let line = line.trim_start();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line
                .strip_prefix("export")
                .filter(|rest| rest.starts_with([' ', '\t']))
                .map_or(line, str::trim_start);
            let invalid_line = || format!("Invalid .env assignment on line {}", index + 1);
            let (name, raw_value) = line.split_once('=').with_context(invalid_line)?;
            let name = name.trim_end();
            ensure!(valid_name(name), "{}", invalid_line());
            ensure!(
                variables.len() + empty + multiline < MAX_ENTRIES,
                "The .env file exceeds 256 variables"
            );

            let mut value = Zeroizing::new(String::new());
            if let Some(quote) = raw_value
                .trim_start()
                .chars()
                .next()
                .filter(|c| matches!(c, '\'' | '"'))
            {
                let mut rest = &raw_value.trim_start()[1..];
                loop {
                    if let Some(tail) = read_quoted_line(rest, quote, &mut value) {
                        let tail = tail.trim();
                        ensure!(
                            tail.is_empty() || tail.starts_with('#'),
                            "{}",
                            invalid_line()
                        );
                        break;
                    }
                    value.push('\n');
                    rest = lines
                        .next()
                        .map(|(_, line)| line)
                        .with_context(invalid_line)?;
                }
            } else {
                value.push_str(unquoted_value(raw_value));
            }
            ensure!(
                value.len() <= MAX_VALUE_BYTES,
                "The value on line {} exceeds 64 KiB",
                index + 1
            );
            if value.is_empty() {
                empty += 1;
            } else if value.contains(['\n', '\r']) {
                multiline += 1;
            } else {
                variables.push(Variable {
                    name: name.to_owned(),
                    value: std::mem::take(&mut *value),
                });
            }
        }
        ensure!(
            !variables.is_empty() || empty + multiline > 0,
            "No .env assignments found"
        );
        Ok(Self {
            variables,
            empty,
            multiline,
        })
    }

    pub(super) fn preview(&self, vault: &Vault, id: Uuid) -> Result<Preview> {
        let items = vault.list_connections()?;
        let mut seen = HashSet::new();
        let rows = self
            .variables
            .iter()
            .enumerate()
            .map(|(index, var)| PreviewRow {
                index,
                name: var.name.clone(),
                conflict: !seen.insert(var.name.to_ascii_lowercase())
                    || items
                        .iter()
                        .any(|item| same_item_name(&item.name, &var.name)),
            })
            .collect();
        Ok(Preview {
            id,
            rows,
            empty: self.empty,
            multiline: self.multiline,
        })
    }

    pub(super) fn commit(
        &self,
        vault: &Vault,
        selection: Vec<usize>,
    ) -> Result<(CommitSummary, Option<UndoBatch>)> {
        ensure!(!selection.is_empty(), "Select at least one variable");
        let selected = selection.iter().copied().collect::<HashSet<_>>();
        ensure!(
            selected.len() == selection.len()
                && selected.iter().all(|index| *index < self.variables.len()),
            "Invalid .env import selection"
        );

        let created = vault.update_document_with_result(|document| {
            let mut created = Vec::with_capacity(selected.len());
            let mut seen = HashSet::new();
            for (index, var) in self.variables.iter().enumerate() {
                if !seen.insert(var.name.to_ascii_lowercase())
                    || !selected.contains(&index)
                    || document
                        .connections
                        .iter()
                        .any(|item| same_item_name(&item.name, &var.name))
                {
                    continue;
                }
                let saved =
                    vault.save_connection_in_document(document, var.to_project(), None, None)?;
                created.push((saved.id, saved.revision));
            }
            Ok(created)
        })?;
        Ok(CommitSummary::from_created(selection.len(), created))
    }
}

impl Variable {
    fn to_project(&self) -> ConnectionInput {
        let mut secrets = SecretBundle::default();
        secrets
            .named_secrets
            .insert(VALUE_FIELD.to_owned(), self.value.clone());
        ConnectionInput {
            name: self.name.clone(),
            item_type: "custom".into(),
            folder: "Env".into(),
            modules: vec![ItemModule {
                kind: "customSecret".into(),
                name: VALUE_FIELD.into(),
                agent_visible: Some(false),
                ..Default::default()
            }],
            secrets,
            id: None,
            enabled: true,
            favorite: false,
            certificate_authority: None,
            oci_bindings: None,
            aws_bindings: None,
            ssh_jump_item_id: None,
            ssh_jump_unresolved: false,
            ssh_auth_type: String::new(),
            login_uris: Vec::new(),
            description: String::new(),
            http_auth_type: String::new(),
            private_key_import_path: String::new(),
            auth_header: String::new(),
            auth_location: String::new(),
            auth_prefix: String::new(),
            api_auth_headers: Vec::new(),
            test_path: String::new(),
            remove_secret_names: Vec::new(),
        }
    }
}

impl CommitSummary {
    pub(super) fn from_created(
        requested: usize,
        created: Vec<(Uuid, u64)>,
    ) -> (Self, Option<UndoBatch>) {
        let added = created.len();
        let batch = (added != 0).then(|| UndoBatch {
            id: Uuid::new_v4(),
            created,
        });
        (
            Self {
                added,
                skipped: requested - added,
                undo_id: batch.as_ref().map(|batch| batch.id),
            },
            batch,
        )
    }
}

impl UndoBatch {
    pub(super) fn undo(&self, vault: &Vault) -> Result<UndoSummary> {
        vault.update_document_with_result(|document| {
            let mut removable = self
                .created
                .iter()
                .filter_map(|&(id, revision)| {
                    document
                        .connections
                        .iter()
                        .any(|item| item.id == id && item.revision == revision)
                        .then_some(id)
                })
                .collect::<HashSet<_>>();
            loop {
                let before = removable.len();
                for item in &document.connections {
                    if !removable.contains(&item.id) {
                        if let Some(jump) = item.ssh_jump_item_id {
                            removable.remove(&jump);
                        }
                    }
                }
                if removable.len() == before {
                    break;
                }
            }
            let mut removed = 0;
            for &(id, _) in &self.created {
                if !removable.contains(&id) {
                    continue;
                }
                if let Some(index) = document.connections.iter().position(|item| item.id == id) {
                    move_connection_to_trash(document, index);
                    removed += 1;
                }
            }
            Ok(UndoSummary {
                removed,
                skipped: self.created.len() - removed,
            })
        })
    }
}

fn valid_name(name: &str) -> bool {
    let mut chars = name.bytes();
    name.len() <= 80
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == b'_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

fn unquoted_value(raw: &str) -> &str {
    let comment = raw.char_indices().find(|(index, ch)| {
        *ch == '#' && (*index == 0 || raw.as_bytes()[index - 1].is_ascii_whitespace())
    });
    comment.map_or(raw, |(index, _)| &raw[..index]).trim()
}

fn read_quoted_line<'a>(line: &'a str, quote: char, value: &mut String) -> Option<&'a str> {
    let mut chars = line.char_indices();
    while let Some((index, ch)) = chars.next() {
        if ch == quote {
            return Some(&line[index + ch.len_utf8()..]);
        }
        if ch == '\\' && quote == '"' {
            match chars.next().map(|(_, next)| next) {
                Some('n') => value.push('\n'),
                Some('r') => value.push('\r'),
                Some('t') => value.push('\t'),
                Some(next @ ('"' | '\\')) => value.push(next),
                next => {
                    value.push('\\');
                    value.extend(next);
                }
            }
        } else {
            value.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_literal_values_without_echoing_invalid_input() {
        let import = EnvImport::parse(
            "\u{feff}# Test values\r\nexport lower='  ${UNEXPANDED} # literal  '\r\n\
             MULTI=\"one\\ntwo\r\nthree\" # ignored\r\nCR=\"one\\rtwo\"\r\nURL=a#b # ignored\r\nEMPTY= # ignored",
        )
        .unwrap();
        assert_eq!(import.empty, 1);
        assert_eq!(import.multiline, 2);
        assert_eq!(import.variables[0].name, "lower");
        assert_eq!(import.variables[0].value, "  ${UNEXPANDED} # literal  ");
        assert_eq!(import.variables[1].value, "a#b");
        assert_eq!(
            EnvImport::parse("ONLY=\"line 1\\nline 2\"")
                .unwrap()
                .multiline,
            1
        );
        assert_eq!(
            EnvImport::parse("OK=fixture\nBROKEN=\"private text")
                .err()
                .unwrap()
                .to_string(),
            "Invalid .env assignment on line 2"
        );
    }

    #[test]
    fn preview_selection_conflicts_and_undo_use_the_real_vault() {
        let root = tempfile::tempdir().unwrap();
        let data_dir = root.path().join("vault");
        let vault = Vault::open(data_dir.clone()).unwrap();
        let path = root.path().join(".env");
        std::fs::write(
            &path,
            "API_KEY=fixture-primary-key\napi_key=fixture-duplicate\nUNCHOSEN=no\nKEPT=fixture-keep\nRACE=fresh",
        )
        .unwrap();
        let import = EnvImport::read(&path).unwrap();
        let preview = import.preview(&vault, Uuid::new_v4()).unwrap();
        assert_eq!(
            preview
                .rows
                .iter()
                .map(|row| row.conflict)
                .collect::<Vec<_>>(),
            [false, true, false, false, false]
        );
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains("fixture-")
        );
        assert!(vault.list_connections().unwrap().is_empty());
        assert!(import.commit(&vault, vec![0, 99]).is_err());
        assert!(vault.list_connections().unwrap().is_empty());

        EnvImport::parse("race=original")
            .unwrap()
            .commit(&vault, vec![0])
            .unwrap();
        let (summary, undo) = import.commit(&vault, vec![0, 1, 3, 4]).unwrap();
        assert_eq!((summary.added, summary.skipped), (2, 2));
        let undo = undo.unwrap();
        assert_eq!(summary.undo_id, Some(undo.id));
        drop(vault);

        let vault = Vault::open(data_dir.clone()).unwrap();
        let projects = vault.list_connections().unwrap();
        let api_key = projects.iter().find(|item| item.name == "API_KEY").unwrap();
        assert_eq!(api_key.folder, "Env");
        assert_eq!(api_key.modules[0].agent_visible, Some(false));
        assert_eq!(
            vault
                .get_connection(api_key.id)
                .unwrap()
                .secrets
                .get("Value"),
            Some("fixture-primary-key")
        );
        assert_eq!(
            vault
                .get_connection(projects.iter().find(|item| item.name == "race").unwrap().id)
                .unwrap()
                .secrets
                .get("Value"),
            Some("original")
        );
        assert!(
            !std::fs::read_to_string(data_dir.join("vault.json"))
                .unwrap()
                .contains("fixture-primary-key")
        );

        let repeat = import.preview(&vault, Uuid::new_v4()).unwrap();
        assert_eq!(
            repeat
                .rows
                .iter()
                .map(|row| row.conflict)
                .collect::<Vec<_>>(),
            [true, true, false, true, true]
        );
        let kept = projects.iter().find(|item| item.name == "KEPT").unwrap();
        vault.set_connection_enabled(kept.id, false).unwrap();
        let summary = undo.undo(&vault).unwrap();
        assert_eq!((summary.removed, summary.skipped), (1, 1));
        assert_eq!(vault.trash_items().unwrap()[0].item.id, api_key.id);
        assert!(vault.get_connection(kept.id).is_ok());
    }
}
