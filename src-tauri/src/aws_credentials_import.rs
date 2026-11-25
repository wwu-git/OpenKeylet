use super::env_import::{CommitSummary, UndoBatch};
use crate::{
    aws,
    model::{AwsModuleBindings, ConnectionInput, ItemModule, SecretBundle},
    vault::{Vault, same_item_name},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};
use uuid::Uuid;
use zeroize::{ZeroizeOnDrop, Zeroizing};

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_PROFILES: usize = 256;
const ACCESS_KEY: &str = "aws_access_key_id";
const SECRET_KEY: &str = "aws_secret_access_key";
const SESSION_TOKEN: &str = "aws_session_token";
const REGION: &str = "aws_region";

#[derive(Serialize, Deserialize, ZeroizeOnDrop)]
struct Profile {
    name: String,
    access_key: String,
    secret_key: String,
    session_token: Option<String>,
    region: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub(super) struct AwsCredentialsImport {
    profiles: Vec<Profile>,
    skipped_profiles: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Preview<'a> {
    id: Uuid,
    rows: Vec<PreviewRow<'a>>,
    skipped_profiles: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRow<'a> {
    index: usize,
    name: &'a str,
    conflict: bool,
    region: Option<&'a str>,
    has_session_token: bool,
}

#[derive(Default)]
struct Section<'a> {
    name: &'a str,
    access_key: Option<&'a str>,
    secret_key: Option<&'a str>,
    session_token: Option<&'a str>,
    region: Option<&'a str>,
    field_indentation: Option<usize>,
    invalid: bool,
}

impl<'a> Section<'a> {
    fn add_field(&mut self, line: &'a str) {
        let Some((key, value)) = line.split_once('=') else {
            self.invalid = true;
            return;
        };
        let key = key.trim().to_ascii_lowercase();
        let field = match key.as_str() {
            ACCESS_KEY => &mut self.access_key,
            SECRET_KEY => &mut self.secret_key,
            SESSION_TOKEN | "aws_security_token" => &mut self.session_token,
            "region" => &mut self.region,
            "credential_process"
            | "credential_source"
            | "role_arn"
            | "source_profile"
            | "web_identity_token_file"
            | "login_session" => {
                self.invalid = true;
                return;
            }
            _ => {
                self.invalid |= key.starts_with("sso_");
                return;
            }
        };
        self.invalid |= field.replace(value.trim()).is_some();
    }

    fn into_profile(self) -> Option<Profile> {
        if self.invalid
            || self.name == "DEFAULT"
            || self.name.starts_with("sso-session ")
            || self.name.starts_with("services ")
        {
            return None;
        }
        let name = format!("AWS ({})", self.name);
        if self.name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            return None;
        }
        let access_key = self.access_key.filter(|key| {
            !key.is_empty()
                && key.len() <= 128
                && key.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })?;
        let secret_key = self.secret_key.filter(|key| valid_value(key))?;
        let session_token = self.session_token.filter(|token| !token.is_empty());
        if session_token.is_some_and(|token| !valid_value(token))
            || (access_key.starts_with("ASIA") && session_token.is_none())
        {
            return None;
        }
        Some(Profile {
            name,
            access_key: access_key.to_owned(),
            secret_key: secret_key.to_owned(),
            session_token: session_token.map(str::to_owned),
            region: self
                .region
                .filter(|region| aws::region_endpoint(region).is_ok())
                .map(str::to_owned),
        })
    }
}

fn valid_value(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= aws::MAX_SECRET_BYTES
        && value.is_ascii()
        && !value.chars().any(char::is_control)
}

impl AwsCredentialsImport {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = path
            .metadata()
            .context("Could not inspect the AWS credentials file")?;
        ensure!(metadata.is_file(), "Choose a regular AWS credentials file");
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "The AWS credentials file exceeds 1 MiB"
        );
        let mut bytes = Zeroizing::new(Vec::new());
        File::open(path)
            .context("Could not open the AWS credentials file")?
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("Could not read the AWS credentials file")?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "The AWS credentials file exceeds 1 MiB"
        );
        Self::parse(std::str::from_utf8(&bytes).context("The AWS credentials file must be UTF-8")?)
    }

    fn parse(text: &str) -> Result<Self> {
        ensure!(
            !text.contains('\0'),
            "The AWS credentials file contains a NUL character"
        );
        let mut sections: Vec<Section<'_>> = Vec::new();
        let mut section_counts = HashMap::new();
        for (index, raw_line) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let indentation = raw_line.chars().take_while(|ch| ch.is_whitespace()).count();
            if let Some(section) = sections.last_mut()
                && section
                    .field_indentation
                    .is_some_and(|previous| indentation > previous)
            {
                section.invalid = true;
                continue;
            }
            if let Some(header) = line.strip_prefix('[') {
                let name = header
                    .strip_suffix(']')
                    .with_context(|| format!("Invalid AWS profile header on line {}", index + 1))?
                    .trim();
                ensure!(
                    !name.contains(['[', ']']),
                    "Invalid AWS profile header on line {}",
                    index + 1
                );
                ensure!(
                    sections.len() < MAX_PROFILES,
                    "The AWS credentials file exceeds 256 profiles"
                );
                *section_counts.entry(name).or_insert(0) += 1;
                sections.push(Section {
                    name,
                    ..Default::default()
                });
            } else {
                let section = sections
                    .last_mut()
                    .context("AWS credential fields must belong to a named profile")?;
                section.field_indentation = Some(indentation);
                section.add_field(line);
            }
        }
        let total = sections.len();
        let profiles: Vec<_> = sections
            .into_iter()
            .filter(|section| section_counts[section.name] == 1)
            .filter_map(Section::into_profile)
            .collect();
        Ok(Self {
            skipped_profiles: total - profiles.len(),
            profiles,
        })
    }

    pub(super) fn preview(&self, vault: &Vault, id: Uuid) -> Result<Preview<'_>> {
        let items = vault.list_connections()?;
        let mut seen = HashSet::new();
        let rows = self
            .profiles
            .iter()
            .enumerate()
            .map(|(index, profile)| PreviewRow {
                index,
                name: &profile.name,
                conflict: !seen.insert(profile.name.to_lowercase())
                    || items
                        .iter()
                        .any(|item| same_item_name(&item.name, &profile.name)),
                region: profile.region.as_deref(),
                has_session_token: profile.session_token.is_some(),
            })
            .collect();
        Ok(Preview {
            id,
            rows,
            skipped_profiles: self.skipped_profiles,
        })
    }

    pub(super) fn commit(
        &self,
        vault: &Vault,
        selection: Vec<usize>,
    ) -> Result<(CommitSummary, Option<UndoBatch>)> {
        ensure!(!selection.is_empty(), "Select at least one AWS profile");
        let selected = selection.iter().copied().collect::<HashSet<_>>();
        ensure!(
            selected.len() == selection.len()
                && selected.iter().all(|index| *index < self.profiles.len()),
            "Invalid AWS profile selection"
        );
        let created = vault.update_document_with_result(|document| {
            let mut created = Vec::with_capacity(selected.len());
            let mut seen = HashSet::new();
            for (index, profile) in self.profiles.iter().enumerate() {
                if !seen.insert(profile.name.to_lowercase())
                    || !selected.contains(&index)
                    || document
                        .connections
                        .iter()
                        .any(|item| same_item_name(&item.name, &profile.name))
                {
                    continue;
                }
                let saved = vault.save_connection_in_document(
                    document,
                    profile.to_project(),
                    None,
                    None,
                )?;
                created.push((saved.id, saved.revision));
            }
            Ok(created)
        })?;
        Ok(CommitSummary::from_created(selection.len(), created))
    }
}

impl Profile {
    fn to_project(&self) -> ConnectionInput {
        let service_url = Uuid::new_v4();
        let mut modules = vec![ItemModule {
            id: service_url,
            kind: "url".into(),
            ..Default::default()
        }];
        let mut secrets = SecretBundle::default();
        let mut bind_secret = |name: &str, value: Option<&str>| {
            let id = Uuid::new_v4();
            modules.push(ItemModule {
                id,
                kind: "customSecret".into(),
                name: name.into(),
                agent_visible: Some(false),
                ..Default::default()
            });
            if let Some(value) = value {
                secrets.named_secrets.insert(name.into(), value.into());
            }
            Some(id)
        };
        let bindings = AwsModuleBindings {
            service_url: Some(service_url),
            region: bind_secret(REGION, self.region.as_deref()),
            access_key_id: bind_secret(ACCESS_KEY, Some(&self.access_key)),
            secret_access_key: bind_secret(SECRET_KEY, Some(&self.secret_key)),
            session_token: bind_secret(SESSION_TOKEN, self.session_token.as_deref()),
        };
        ConnectionInput {
            name: self.name.clone(),
            item_type: "api".into(),
            folder: "Cloud".into(),
            modules,
            secrets,
            id: None,
            enabled: true,
            favorite: false,
            certificate_authority: None,
            oci_bindings: None,
            aws_bindings: Some(bindings),
            ssh_jump_item_id: None,
            ssh_jump_unresolved: false,
            ssh_auth_type: String::new(),
            login_uris: Vec::new(),
            description: String::new(),
            http_auth_type: aws::AUTH_MODE.into(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_provider::check_aws_configuration;

    #[test]
    fn selected_profiles_are_bound_and_can_pass_local_check_after_configuration() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("vault");
        let vault = Vault::open(directory.clone()).unwrap();
        let path = root.path().join("credentials");
        let input = "\u{feff}[default]\naws_access_key_id=AKIDEXAMPLE\naws_secret_access_key=fixture-default-secret\nregion=us-east-1\n\n[temporary]\naws_access_key_id=ASIATEMPORARYTEST\naws_secret_access_key=fixture-temporary-secret\naws_session_token=fixture-session==\nregion=unrecognized-region\n";
        std::fs::write(&path, input).unwrap();
        let vault_path = directory.join("vault.json");
        let before = std::fs::read(&vault_path).unwrap();
        let preview_id = Uuid::new_v4();
        let import = AwsCredentialsImport::read(&path).unwrap();
        let preview = serde_json::to_value(import.preview(&vault, preview_id).unwrap()).unwrap();
        assert_eq!(
            preview,
            serde_json::json!({
                "id": preview_id,
                "rows": [
                    {"index":0, "name":"AWS (default)", "region":"us-east-1", "hasSessionToken":false, "conflict":false},
                    {"index":1, "name":"AWS (temporary)", "region":null, "hasSessionToken":true, "conflict":false}
                ],
                "skippedProfiles": 0
            })
        );
        drop(vault.protect_local_state(&import).unwrap());
        assert_eq!(std::fs::read(&vault_path).unwrap(), before);
        assert!(import.commit(&vault, vec![1, 1]).is_err());
        assert!(import.commit(&vault, vec![2]).is_err());
        assert_eq!(std::fs::read(&vault_path).unwrap(), before);

        let (commit, _) = import.commit(&vault, vec![1]).unwrap();
        assert_eq!(serde_json::to_value(commit).unwrap()["added"], 1);
        let projects = vault.list_decrypted_connections().unwrap();
        assert_eq!(projects.len(), 1);
        let project = &projects[0];
        assert_eq!(project.stored.name, "AWS (temporary)");
        assert_eq!(project.stored.http_auth_type, aws::AUTH_MODE);
        assert_eq!(project.stored.base_url, "");
        assert!(check_aws_configuration(&project.stored, &project.secrets).is_err());
        let bindings = project.stored.aws_bindings.clone().unwrap();
        for (binding, name, value) in [
            (bindings.access_key_id, ACCESS_KEY, "ASIATEMPORARYTEST"),
            (
                bindings.secret_access_key,
                SECRET_KEY,
                "fixture-temporary-secret",
            ),
            (bindings.session_token, SESSION_TOKEN, "fixture-session=="),
        ] {
            let module = project
                .stored
                .modules
                .iter()
                .find(|module| Some(module.id) == binding)
                .unwrap();
            assert_eq!(module.name, name);
            assert_eq!(module.value, "");
            assert!(!module.agent_visible());
            assert_eq!(project.secrets.get(name), Some(value));
        }
        let disk = std::fs::read_to_string(&vault_path).unwrap();
        for value in [
            "AKIDEXAMPLE",
            "fixture-default-secret",
            "ASIATEMPORARYTEST",
            "fixture-temporary-secret",
            "fixture-session==",
        ] {
            assert!(!disk.contains(value));
        }

        let mut edit = import.profiles[1].to_project();
        edit.id = Some(project.stored.id);
        edit.modules = project.stored.modules.clone();
        edit.aws_bindings = Some(bindings.clone());
        edit.secrets = SecretBundle::default();
        edit.modules
            .iter_mut()
            .find(|module| Some(module.id) == bindings.service_url)
            .unwrap()
            .value = "https://ec2.us-west-2.amazonaws.com/".to_owned();
        edit.secrets
            .named_secrets
            .insert(REGION.into(), "us-west-2".into());
        vault.save_connection(edit).unwrap();
        let updated = vault.get_connection(project.stored.id).unwrap();
        assert!(
            check_aws_configuration(&updated.stored, &updated.secrets)
                .unwrap()
                .contains("No request was sent")
        );
    }

    #[test]
    fn duplicate_profiles_keys_and_external_credential_sources_do_not_mix() {
        let import = AwsCredentialsImport::parse(concat!(
            "[missing]\naws_access_key_id=AKIDEXAMPLE\n",
            "[broken]\naws_access_key_id=ASIAEXAMPLE\naws_secret_access_key=fixture-secret\n",
            "[sso]\naws_access_key_id=AKIDEXAMPLE\naws_secret_access_key=fixture-secret\nsso_session=fixture\n",
            "[role]\naws_access_key_id=AKIDEXAMPLE\naws_secret_access_key=fixture-secret\nrole_arn=fixture-role\n",
            "[process]\naws_access_key_id=AKIDEXAMPLE\naws_secret_access_key=fixture-secret\ncredential_process=synthetic-process\n",
            "[duplicate]\naws_access_key_id=AKIDEXAMPLE\n",
            "[duplicate]\naws_secret_access_key=fixture-secret\n",
            "[duplicate-key]\naws_access_key_id=AKIDEXAMPLE\nAWS_ACCESS_KEY_ID=AKIDSECOND\naws_secret_access_key=fixture-secret\n",
            "[DEFAULT]\naws_access_key_id=AKIDEXAMPLE\naws_secret_access_key=fixture-secret\n",
            "[valid]\naws_access_key_id=AKIDVALID\naws_secret_access_key=fixture-valid\n",
        )).unwrap();
        assert_eq!(import.skipped_profiles, 9);
        assert_eq!(import.profiles.len(), 1);
        assert_eq!(import.profiles[0].name, "AWS (valid)");
    }

    #[test]
    fn multiline_options_cannot_create_static_credentials_or_profiles() {
        let import = AwsCredentialsImport::parse(concat!(
            "[multiline]\nother =\n",
            "  aws_access_key_id=AKIDEXAMPLE\n  aws_secret_access_key=fixture-secret\n",
            "  [nested-profile]\n  aws_access_key_id=AKIDEXAMPLE\n  aws_secret_access_key=fixture-secret\n",
            "[unicode-indentation]\n\u{2000}other=\n  aws_access_key_id=AKIDEXAMPLE\n  aws_secret_access_key=fixture-secret\n",
            "[valid]\n  aws_access_key_id=AKIDVALID\n  aws_secret_access_key=fixture-valid\n",
        )).unwrap();
        assert_eq!(import.skipped_profiles, 2);
        assert_eq!(import.profiles.len(), 1);
        assert_eq!(import.profiles[0].name, "AWS (valid)");
    }

    #[test]
    fn duplicate_at_commit_keeps_original_secret_and_has_no_undo() {
        let root = tempfile::tempdir().unwrap();
        let vault = Vault::open(root.path().join("vault")).unwrap();
        let latest = AwsCredentialsImport::parse(
            "[same]\naws_access_key_id=AKIDLATEST\naws_secret_access_key=fixture-latest\n",
        )
        .unwrap();
        assert!(!latest.preview(&vault, Uuid::new_v4()).unwrap().rows[0].conflict);
        let original = AwsCredentialsImport::parse(
            "[SAME]\naws_access_key_id=AKIDORIGINAL\naws_secret_access_key=fixture-original\n",
        )
        .unwrap();
        original.commit(&vault, vec![0]).unwrap();
        assert!(latest.preview(&vault, Uuid::new_v4()).unwrap().rows[0].conflict);
        let (summary, undo) = latest.commit(&vault, vec![0]).unwrap();
        assert!(undo.is_none());
        assert_eq!(serde_json::to_value(summary).unwrap()["skipped"], 1);
        let items = vault.list_decrypted_connections().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].secrets.get(SECRET_KEY), Some("fixture-original"));
    }
}
