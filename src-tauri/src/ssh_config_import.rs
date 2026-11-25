use super::env_import::{CommitSummary, UndoBatch};
use crate::{
    model::{ConnectionInput, ItemModule, SecretBundle},
    vault::{Vault, same_item_name},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    net::Ipv6Addr,
    path::Path,
};
use uuid::Uuid;
use zeroize::Zeroizing;

const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_HOSTS: usize = 256;

#[derive(Clone, Serialize, Deserialize)]
enum Route {
    Direct,
    Jump(String),
    Unresolved,
}

#[derive(Default)]
struct Options {
    host: Option<String>,
    port: Option<u16>,
    user: Option<String>,
    route: Option<Route>,
}

struct Block {
    aliases: HashSet<String>,
    options: Options,
}

#[derive(Serialize, Deserialize)]
struct Host {
    name: String,
    host: String,
    port: u16,
    user: String,
    route: Route,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct SshConfigImport {
    hosts: Vec<Host>,
}

#[derive(Serialize)]
pub(super) struct Preview {
    id: Uuid,
    rows: Vec<PreviewRow>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewRow {
    index: usize,
    name: String,
    host: String,
    port: u16,
    user: String,
    jump: Option<String>,
    route: &'static str,
    jump_index: Option<usize>,
    conflict: bool,
}

impl SshConfigImport {
    pub(super) fn read(path: &Path) -> Result<Self> {
        let metadata = path
            .metadata()
            .context("Could not inspect the selected SSH config")?;
        ensure!(metadata.is_file(), "Choose a regular SSH config file");
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "The SSH config exceeds 1 MiB"
        );
        let file = File::open(path).context("Could not open the selected SSH config")?;
        ensure!(
            file.metadata()?.is_file(),
            "Choose a regular SSH config file"
        );
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("Could not read the selected SSH config")?;
        ensure!(
            bytes.len() as u64 <= MAX_FILE_BYTES,
            "The SSH config exceeds 1 MiB"
        );
        Self::parse(std::str::from_utf8(&bytes).context("The SSH config must be UTF-8")?)
    }

    pub(crate) fn parse(text: &str) -> Result<Self> {
        ensure!(
            !text.contains('\0'),
            "The SSH config contains a NUL character"
        );
        let mut names = Vec::new();
        let mut seen = HashMap::new();
        let mut blocks = vec![Block {
            aliases: HashSet::new(),
            options: Options::default(),
        }];

        for (index, raw) in text.trim_start_matches('\u{feff}').lines().enumerate() {
            let line = raw.trim_ascii();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let end = line
                .find(|ch: char| ch.is_ascii_whitespace() || ch == '=')
                .unwrap_or(line.len());
            ensure!(
                end > 0 && line[..end].bytes().all(|byte| byte.is_ascii_alphanumeric()),
                "Unsupported SSH config keyword syntax on line {}",
                index + 1
            );
            let directive = line[..end].to_ascii_lowercase();
            let tail = line[end..].trim_ascii_start();
            let args = tail.strip_prefix('=').unwrap_or(tail).trim_ascii_start();
            let invalid = || format!("Invalid SSH config directive on line {}", index + 1);
            match directive.as_str() {
                "include" | "match" | "proxycommand" => {
                    bail!(
                        "Unsupported SSH routing directive on line {}. Choose a config with explicit Host entries.",
                        index + 1
                    );
                }
                "canonicalizehostname" => {
                    ensure!(
                        one_value(args).is_some_and(|value| value.eq_ignore_ascii_case("no")),
                        "Unsupported SSH hostname canonicalization on line {}",
                        index + 1
                    );
                }
                "host" => {
                    let patterns = words(args).collect::<Vec<_>>();
                    ensure!(!patterns.is_empty(), "{}", invalid());
                    ensure!(
                        blocks.len() <= 1024,
                        "The SSH config exceeds 1024 Host blocks"
                    );
                    let mut aliases = HashSet::new();
                    if patterns.as_slice() != ["*"] {
                        for pattern in patterns {
                            let alias = unquote(pattern).filter(|value| valid_alias(value))
                                .with_context(|| format!("Unsupported Host pattern on line {}. Use an explicit alias or Host *.", index + 1))?;
                            let key = alias.to_ascii_lowercase();
                            if let Some(previous) = seen.get(&key) {
                                ensure!(
                                    *previous == alias,
                                    "Host aliases differ only by case on line {}",
                                    index + 1
                                );
                            } else {
                                ensure!(
                                    names.len() < MAX_HOSTS,
                                    "The SSH config exceeds 256 host aliases"
                                );
                                seen.insert(key, alias);
                                names.push(alias.to_owned());
                            }
                            aliases.insert(alias.to_owned());
                        }
                    }
                    blocks.push(Block {
                        aliases,
                        options: Options::default(),
                    });
                }
                "hostname" | "port" | "user" | "proxyjump" => {
                    let value = one_value(args).with_context(invalid)?;
                    let options = &mut blocks.last_mut().expect("global SSH config block").options;
                    match directive.as_str() {
                        "hostname" => {
                            let host = value
                                .strip_prefix('[')
                                .and_then(|value| value.strip_suffix(']'))
                                .unwrap_or(value);
                            ensure!(
                                host.parse::<Ipv6Addr>().is_ok() || valid_hostname(host),
                                "{}",
                                invalid()
                            );
                            options.host.get_or_insert_with(|| host.to_owned());
                        }
                        "port" => {
                            let port = value
                                .parse::<u16>()
                                .ok()
                                .filter(|port| *port != 0)
                                .with_context(invalid)?;
                            options.port.get_or_insert(port);
                        }
                        "user" => {
                            ensure!(valid_alias(value), "{}", invalid());
                            options.user.get_or_insert_with(|| value.to_owned());
                        }
                        _ => {
                            let quoted =
                                words(args).next().is_some_and(|word| word.starts_with('"'));
                            let route = if args.eq_ignore_ascii_case("none") {
                                Route::Direct
                            } else if !quoted
                                && !value.eq_ignore_ascii_case("none")
                                && valid_alias(value)
                            {
                                Route::Jump(value.to_owned())
                            } else {
                                Route::Unresolved
                            };
                            options.route.get_or_insert(route);
                        }
                    }
                }
                _ => {}
            }
        }
        ensure!(
            !names.is_empty(),
            "No explicit Host aliases found in the SSH config"
        );
        let hosts = names
            .into_iter()
            .map(|name| {
                let options = || {
                    blocks
                        .iter()
                        .filter(|block| block.aliases.is_empty() || block.aliases.contains(&name))
                        .map(|block| &block.options)
                };
                Host {
                    host: options()
                        .find_map(|option| option.host.clone())
                        .unwrap_or_else(|| name.clone()),
                    port: options().find_map(|option| option.port).unwrap_or(22),
                    user: options()
                        .find_map(|option| option.user.clone())
                        .unwrap_or_default(),
                    route: options()
                        .find_map(|option| option.route.clone())
                        .unwrap_or(Route::Direct),
                    name,
                }
            })
            .collect();
        Ok(Self { hosts })
    }

    fn jump_index(&self, index: usize) -> Option<usize> {
        let Route::Jump(alias) = &self.hosts[index].route else {
            return None;
        };
        self.hosts.iter().enumerate().find_map(|(candidate, host)| {
            (candidate != index && matches!(host.route, Route::Direct) && host.name == *alias)
                .then_some(candidate)
        })
    }

    pub(super) fn preview(&self, vault: &Vault, id: Uuid) -> Result<Preview> {
        let existing = vault.list_connections()?;
        let conflict = |name: &str| existing.iter().any(|item| same_item_name(&item.name, name));
        let rows = self
            .hosts
            .iter()
            .enumerate()
            .map(|(index, host)| {
                let jump_index = self
                    .jump_index(index)
                    .filter(|candidate| !conflict(&self.hosts[*candidate].name));
                PreviewRow {
                    index,
                    name: host.name.clone(),
                    host: host.host.clone(),
                    port: host.port,
                    user: host.user.clone(),
                    jump: match &host.route {
                        Route::Jump(alias) => Some(alias.clone()),
                        _ => None,
                    },
                    route: match (&host.route, jump_index) {
                        (Route::Direct, _) => "direct",
                        (_, Some(_)) => "jump",
                        _ => "unresolved",
                    },
                    jump_index,
                    conflict: conflict(&host.name),
                }
            })
            .collect();
        Ok(Preview { id, rows })
    }

    pub(crate) fn commit(
        &self,
        vault: &Vault,
        selection: Vec<usize>,
    ) -> Result<(CommitSummary, Option<UndoBatch>)> {
        ensure!(!selection.is_empty(), "Select at least one SSH host");
        let selected = selection.iter().copied().collect::<HashSet<_>>();
        ensure!(
            selected.len() == selection.len()
                && selected.iter().all(|index| *index < self.hosts.len()),
            "Invalid SSH config selection"
        );
        let created = vault.update_document_with_result(|document| {
            let first_new = document.connections.len();
            let mut created = vec![None; self.hosts.len()];
            for (index, host) in self.hosts.iter().enumerate() {
                if !selected.contains(&index)
                    || document
                        .connections
                        .iter()
                        .any(|item| same_item_name(&item.name, &host.name))
                {
                    continue;
                }
                let saved =
                    vault.save_connection_in_document(document, host.to_project(), None, None)?;
                created[index] = Some((saved.id, saved.revision));
            }
            let created_indices = created
                .iter()
                .enumerate()
                .filter_map(|(index, item)| item.is_some().then_some(index));
            for (index, item) in created_indices.zip(&mut document.connections[first_new..]) {
                if matches!(self.hosts[index].route, Route::Direct) {
                    continue;
                }
                item.ssh_jump_item_id = self
                    .jump_index(index)
                    .and_then(|index| created[index])
                    .map(|(id, _)| id);
                item.ssh_jump_unresolved = item.ssh_jump_item_id.is_none();
            }
            Ok(created.into_iter().flatten().collect())
        })?;
        Ok(CommitSummary::from_created(selection.len(), created))
    }
}

impl Host {
    fn to_project(&self) -> ConnectionInput {
        let description = match &self.route {
            Route::Direct => String::new(),
            Route::Jump(alias) => format!("Imported SSH config. ProxyJump: {alias}"),
            Route::Unresolved => {
                "Imported SSH config. Unsupported ProxyJump requires a manual route.".to_owned()
            }
        };
        let mut secrets = SecretBundle::default();
        if !self.user.is_empty() {
            secrets
                .named_secrets
                .insert("username".to_owned(), self.user.clone());
        }
        let module = |kind: &str, value: String| ItemModule {
            kind: kind.to_owned(),
            value,
            ..Default::default()
        };
        ConnectionInput {
            name: self.name.clone(),
            item_type: "ssh".into(),
            folder: "SSH".into(),
            description,
            modules: vec![
                module("host", self.host.clone()),
                module("port", self.port.to_string()),
                module("username", String::new()),
                module("password", String::new()),
            ],
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

fn words(value: &str) -> impl Iterator<Item = &str> {
    value
        .split_ascii_whitespace()
        .take_while(|word| !word.starts_with('#'))
}

fn unquote(value: &str) -> Option<&str> {
    if let Some(inner) = value.strip_prefix('"') {
        inner.strip_suffix('"')
    } else {
        Some(value)
    }
}

fn one_value(value: &str) -> Option<&str> {
    let mut parts = words(value);
    let token = unquote(parts.next()?)?;
    (!token.is_empty() && parts.next().is_none()).then_some(token)
}

fn valid_alias(value: &str) -> bool {
    valid_host_token(value, 80)
}

fn valid_hostname(value: &str) -> bool {
    valid_host_token(value, 253)
}

fn valid_host_token(value: &str, limit: usize) -> bool {
    value.len() <= limit
        && !value.is_empty()
        && !value.starts_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_first_match_and_refuses_ambiguous_routing() {
        let import = SshConfigImport::parse(
            "User default\nHost hop target\n Port 2200\nHost target\n User ignored\n HostName internal.example\n ProxyJump hop\nHost *\n Port 2201\nHost quoted\n ProxyJump \"none\"\nHost comment\n ProxyJump none # comment\nHost direct\n ProxyJump NONE\nHost different\n ProxyJump HOP",
        ).unwrap();
        assert_eq!(import.hosts[0].name, "hop");
        assert_eq!(import.hosts[1].host, "internal.example");
        assert_eq!(import.hosts[1].port, 2200);
        assert_eq!(import.hosts[1].user, "default");
        assert_eq!(import.jump_index(1), Some(0));
        assert!(matches!(import.hosts[2].route, Route::Unresolved));
        assert!(matches!(import.hosts[3].route, Route::Unresolved));
        assert!(matches!(import.hosts[4].route, Route::Direct));
        assert_eq!(import.jump_index(5), None);
        let spaced = SshConfigImport::parse("Host alias\n ProxyJump none\u{a0}").unwrap();
        assert!(matches!(spaced.hosts[0].route, Route::Unresolved));
        for input in [
            "Host alias\n Include config.d/*",
            "Host alias\n \"Include\" config.d/*",
            "Host alias\n \"ProxyJump\" jump",
            "Host alias\n Match all",
            "Host alias\n ProxyCommand none",
            "Host alias\n CanonicalizeHostname yes",
            "Host *.internal\n ProxyJump jump\nHost alias",
            "Host Demo\nHost demo",
        ] {
            assert!(SshConfigImport::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn writes_only_selected_hosts_and_retains_blocked_routes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("config");
        std::fs::write(&path, "Host hop\n HostName one.example\n IdentityFile /private/never-read-this-key\nHost target\n HostName two.example\n Port 2222\n User operator\n ProxyJump hop\nHost missing\n ProxyJump elsewhere\nHost unselected").unwrap();
        let vault_path = root.path().join("vault");
        let vault = Vault::open(vault_path.clone()).unwrap();
        let imported = SshConfigImport::read(&path).unwrap();
        let preview = imported.preview(&vault, Uuid::new_v4()).unwrap();
        assert_eq!(preview.rows[1].route, "jump");
        assert_eq!(preview.rows[1].jump_index, Some(0));
        assert_eq!(preview.rows[2].route, "unresolved");
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains("never-read-this-key")
        );
        assert!(vault.list_connections().unwrap().is_empty());
        let (summary, _) = imported.commit(&vault, vec![0, 1, 2]).unwrap();
        assert_eq!(summary.added, 3);
        drop(vault);

        let vault = Vault::open(vault_path).unwrap();
        let items = vault.list_connections().unwrap();
        let hop = vault.get_connection(items[0].id).unwrap();
        let target = vault.get_connection(items[1].id).unwrap();
        let missing = vault.get_connection(items[2].id).unwrap();
        assert_eq!(hop.stored.host, "one.example");
        assert!(hop.secrets.private_key.is_none());
        assert!(vault.ssh_host_trust().unwrap().is_empty());
        assert_eq!(target.stored.port, 2222);
        assert_eq!(target.secrets.get("username"), Some("operator"));
        assert_eq!(target.stored.ssh_jump_item_id, Some(hop.stored.id));
        assert!(vault.ssh_jump_connection(&target).is_err());
        assert!(missing.stored.ssh_jump_unresolved);
        assert!(vault.ssh_jump_connection(&missing).is_err());
        assert!(imported.preview(&vault, Uuid::new_v4()).unwrap().rows[1].conflict);

        let fresh = Vault::open(root.path().join("fresh")).unwrap();
        let (_, undo) = imported.commit(&fresh, vec![0, 1]).unwrap();
        let items = fresh.list_connections().unwrap();
        fresh.set_connection_enabled(items[1].id, false).unwrap();
        let summary = undo.unwrap().undo(&fresh).unwrap();
        assert_eq!(summary.removed, 0);
        assert_eq!(fresh.list_connections().unwrap().len(), 2);

        let fresh = Vault::open(root.path().join("target-alone")).unwrap();
        imported.commit(&fresh, vec![1]).unwrap();
        let target = fresh
            .get_connection(fresh.list_connections().unwrap()[0].id)
            .unwrap();
        assert!(target.stored.ssh_jump_unresolved);
        assert!(target.stored.ssh_jump_item_id.is_none());
    }
}
