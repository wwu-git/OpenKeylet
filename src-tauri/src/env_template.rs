use crate::{
    executor::{SshResponse, execute_local},
    policy::redact_values,
    vault::Vault,
};
use anyhow::{Context, Result, ensure};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

const MAX_TEMPLATE_BYTES: usize = 64 * 1024;
const MAX_BINDINGS: usize = 256;
const INVALID_BINDING: &str = "Invalid OpenKeylet .env template binding";
const PREFIX: &str = "{{openkeylet:";

#[cfg(any(feature = "gui", test))]
pub(crate) fn placeholder(item: Uuid, module: Uuid) -> String {
    format!("{PREFIX}{item}/{module}}}}}")
}

struct Binding<'a> {
    name: &'a str,
    item: Uuid,
    module: Uuid,
}

fn bindings(template: &str) -> Result<Vec<Binding<'_>>> {
    ensure!(
        !template.is_empty() && template.len() <= MAX_TEMPLATE_BYTES,
        "Invalid OpenKeylet .env template size"
    );
    let mut bindings = Vec::new();
    let mut names = HashSet::new();
    for line in template.lines() {
        ensure!(
            bindings.len() < MAX_BINDINGS,
            "Too many .env template bindings"
        );
        let (name, reference) = line.split_once('=').context(INVALID_BINDING)?;
        ensure!(
            !name.is_empty()
                && !name.as_bytes()[0].is_ascii_digit()
                && name
                    .bytes()
                    .all(|byte| byte == b'_' || byte.is_ascii_alphanumeric()),
            INVALID_BINDING
        );
        ensure!(
            names.insert(name.to_ascii_uppercase()),
            "Duplicate .env template variable"
        );
        let (item, module) = reference
            .strip_prefix(PREFIX)
            .and_then(|reference| reference.strip_suffix("}}"))
            .and_then(|reference| reference.split_once('/'))
            .context(INVALID_BINDING)?;
        ensure!(item.len() == 36 && module.len() == 36, INVALID_BINDING);
        let item = Uuid::parse_str(item).ok().context(INVALID_BINDING)?;
        let module = Uuid::parse_str(module).ok().context(INVALID_BINDING)?;
        bindings.push(Binding { name, item, module });
    }
    ensure!(!bindings.is_empty(), INVALID_BINDING);
    Ok(bindings)
}

#[derive(Default)]
pub(crate) struct Environment(HashMap<String, String>);

impl Drop for Environment {
    fn drop(&mut self) {
        self.0.values_mut().for_each(Zeroize::zeroize);
    }
}

pub(crate) fn resolve(vault: &Vault, template: &str) -> Result<Environment> {
    let mut environment = Environment::default();
    for binding in bindings(template)? {
        let connection = vault.agent_connection(binding.item)?;
        let module_name = connection
            .stored
            .modules
            .iter()
            .find(|module| module.id == binding.module && module.kind == "customSecret")
            .and_then(|module| module.secret_name())
            .context(".env template module is unavailable")?;
        let value = connection
            .secrets
            .get(module_name)
            .filter(|value| !value.is_empty() && !value.contains('\0'))
            .context(".env template value is unavailable")?;
        environment
            .0
            .insert(binding.name.to_owned(), value.to_owned());
    }
    Ok(environment)
}

pub(crate) async fn execute(
    environment: &Environment,
    command: &str,
    cwd: Option<&str>,
    timeout_seconds: Option<u64>,
    stdin: Option<&str>,
) -> Result<SshResponse> {
    let mut result =
        execute_local(None, command, cwd, timeout_seconds, &environment.0, stdin).await?;
    let secrets = Zeroizing::new(environment.0.values().cloned().collect::<Vec<_>>());
    result.stdout = redact_values(result.stdout, &secrets);
    result.stderr = redact_values(result.stderr, &secrets);
    Ok(result)
}
