use super::{Entry, blank, field};
use crate::model::ItemModule;
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use url::Url;
use zeroize::Zeroizing;

const MAX_COMMAND_BYTES: usize = 32 * 1024;
const INVALID: &str = "CURL_IMPORT_INVALID";
const UNSUPPORTED: &str = "CURL_IMPORT_UNSUPPORTED";

enum CurlAuthentication {
    Bearer(Zeroizing<String>),
    Basic {
        username: Zeroizing<String>,
        password: Zeroizing<String>,
    },
}

#[derive(Serialize)]
pub(crate) struct CurlPreview {
    target: String,
    authentication: &'static str,
}

pub(crate) fn parse_curl(command: &str) -> Result<(Entry, CurlPreview)> {
    ensure!(command.len() <= MAX_COMMAND_BYTES, INVALID);
    let normalized = Zeroizing::new(
        command
            .trim_matches([' ', '\t', '\r', '\n'])
            .replace("\r\n", "\n"),
    );
    validate_shell_words(&normalized)?;
    let normalized = Zeroizing::new(normalized.replace("\\\n", ""));
    let mut lexer = shlex::Shlex::new(&normalized);
    let words = Zeroizing::new(lexer.by_ref().collect::<Vec<_>>());
    ensure!(!lexer.had_error, INVALID);
    let mut args = words.iter().map(String::as_str);
    ensure!(matches!(args.next(), Some("curl" | "curl.exe")), INVALID);

    let mut endpoint = None;
    let mut authentication = None;
    while let Some(arg) = args.next() {
        let (option, inline) = arg
            .strip_prefix("--")
            .and_then(|_| arg.split_once('='))
            .map_or((arg, None), |(option, value)| (option, Some(value)));
        match option {
            "-H" | "--header" | "-u" | "--user" => {
                ensure!(authentication.is_none(), INVALID);
                let value = inline.or_else(|| args.next()).context(INVALID)?;
                authentication = Some(match option {
                    "-H" | "--header" => parse_header(value)?,
                    _ => parse_basic(value)?,
                });
            }
            "--url" => {
                ensure!(endpoint.is_none(), INVALID);
                endpoint = Some(inline.or_else(|| args.next()).context(INVALID)?);
            }
            _ if !arg.starts_with('-') => {
                ensure!(endpoint.is_none(), INVALID);
                endpoint = Some(arg);
            }
            _ => bail!(UNSUPPORTED),
        }
    }
    let target = service_origin(endpoint.context(INVALID)?)?;
    build_entry(target, authentication.context(INVALID)?)
}

fn validate_shell_words(command: &str) -> Result<()> {
    let mut quote = None;
    let mut chars = command.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if quote != Some('\'') => {
                let next = chars.next().context(INVALID)?;
                ensure!(!next.is_control() || matches!(next, '\t' | '\n'), INVALID);
            }
            '\'' | '"' if quote.is_none() => quote = Some(ch),
            '\'' | '"' if quote == Some(ch) => quote = None,
            '$' | '`' if quote != Some('\'') => bail!(UNSUPPORTED),
            ';' | '|' | '&' | '<' | '>' | '(' | ')' | '#' if quote.is_none() => {
                bail!(UNSUPPORTED);
            }
            ch if ch.is_control() && ch != '\t' => bail!(INVALID),
            _ => {}
        }
    }
    ensure!(quote.is_none(), INVALID);
    Ok(())
}

fn service_origin(raw: &str) -> Result<Url> {
    let authority = raw
        .get(8..)
        .and_then(|rest| rest.split('/').next())
        .filter(|authority| !authority.is_empty() && !authority.contains('@'));
    ensure!(
        raw.get(..8)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
            && authority.is_some()
            && !raw.chars().any(|ch| ch.is_whitespace() || ch == '\\'),
        UNSUPPORTED
    );
    let mut target = Url::parse(raw).context(INVALID)?;
    ensure!(
        target.host_str().is_some()
            && target.username().is_empty()
            && target.password().is_none()
            && target.query().is_none()
            && target.fragment().is_none(),
        UNSUPPORTED
    );
    target.set_path("/");
    Ok(target)
}

fn parse_header(input: &str) -> Result<CurlAuthentication> {
    let (name, value) = input.split_once(':').context(INVALID)?;
    ensure!(name.eq_ignore_ascii_case("Authorization"), UNSUPPORTED);
    let (scheme, credentials) = value
        .trim()
        .split_once(char::is_whitespace)
        .context(INVALID)?;
    let credentials = credentials.trim();
    if scheme.eq_ignore_ascii_case("Bearer") {
        let token = credentials.trim_end_matches('=');
        ensure!(
            !token.is_empty()
                && token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b)),
            INVALID
        );
        return Ok(CurlAuthentication::Bearer(Zeroizing::new(
            credentials.to_owned(),
        )));
    }
    ensure!(scheme.eq_ignore_ascii_case("Basic"), UNSUPPORTED);
    let decoded = Zeroizing::new(
        STANDARD
            .decode(credentials)
            .map_err(|_| anyhow::anyhow!(INVALID))?,
    );
    parse_basic(std::str::from_utf8(&decoded).map_err(|_| anyhow::anyhow!(INVALID))?)
}

fn parse_basic(input: &str) -> Result<CurlAuthentication> {
    let (username, password) = input.split_once(':').context(INVALID)?;
    ensure!(
        !username.trim().is_empty()
            && !password.trim().is_empty()
            && !input.chars().any(char::is_control),
        INVALID
    );
    Ok(CurlAuthentication::Basic {
        username: Zeroizing::new(username.to_owned()),
        password: Zeroizing::new(password.to_owned()),
    })
}

fn build_entry(target: Url, authentication: CurlAuthentication) -> Result<(Entry, CurlPreview)> {
    let host = target.host_str().context(INVALID)?;
    let name = host.chars().take(80).collect::<String>();
    let mut entry = blank("api", &name)?;
    entry.0.modules.push(ItemModule {
        kind: "url".to_owned(),
        value: target.to_string(),
        ..Default::default()
    });
    let mode = match authentication {
        CurlAuthentication::Bearer(token) => {
            field(&mut entry, "apiCredential", &token)?;
            "bearer"
        }
        CurlAuthentication::Basic { username, password } => {
            field(&mut entry, "username", &username)?;
            field(&mut entry, "password", &password)?;
            "basic"
        }
    };
    entry.0.http_auth_type = mode.to_owned();
    entry.0.auth_header = "Authorization".to_owned();
    entry.0.auth_location = "header".to_owned();
    Ok((
        entry,
        CurlPreview {
            target: target.into(),
            authentication: mode,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{api_provider, vault::Vault};

    #[test]
    fn previews_metadata_then_merges_encrypted_api_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().into()).unwrap();
        let (entry, preview) = parse_curl(
            "curl \\\r\n  'https://example.test:8443/private-route' \\\r\n  -H 'Authorization: Bearer test-secret-token'",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            serde_json::json!({"target":"https://example.test:8443/","authentication":"bearer"})
        );
        assert!(vault.list_connections().unwrap().is_empty());

        let (basic, preview) = parse_curl(
            r#"curl --url=https://example.test --user='test-user:quoted; test-secret:password'"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(preview).unwrap(),
            serde_json::json!({"target":"https://example.test/","authentication":"basic"})
        );
        let staged = vault.protect_local_state(&vec![entry, basic]).unwrap();
        assert!(vault.list_connections().unwrap().is_empty());
        assert_eq!(
            vault
                .merge_connections(vault.unprotect_local_state(&staged).unwrap())
                .unwrap()
                .added,
            2
        );
        let saved = vault.list_decrypted_connections().unwrap();
        let bearer = &saved[0];
        let basic = &saved[1];
        assert_eq!(bearer.stored.http_auth_type, "bearer");
        assert_eq!(bearer.stored.base_url, "https://example.test:8443/");
        assert_eq!(
            bearer.secrets.get("apiCredential"),
            Some("test-secret-token")
        );
        assert_eq!(basic.stored.http_auth_type, "basic");
        assert_eq!(basic.secrets.get("username"), Some("test-user"));
        assert_eq!(
            basic.secrets.get("password"),
            Some("quoted; test-secret:password")
        );
        for connection in saved {
            assert!(matches!(
                api_provider::execution_plan(
                    &connection.stored,
                    &connection.secrets,
                    &Url::parse(&connection.stored.base_url).unwrap(),
                )
                .unwrap(),
                api_provider::ExecutionPlan::Generic
            ));
            assert!(
                connection
                    .stored
                    .modules
                    .iter()
                    .filter(|module| module.secret_name().is_some())
                    .all(|module| module.agent_visible == Some(false) && module.value.is_empty())
            );
        }
        let disk = std::fs::read_to_string(directory.path().join("vault.json")).unwrap();
        for private_value in ["private-route", "test-secret", "test-user"] {
            assert!(!disk.contains(private_value));
        }
    }

    #[test]
    fn reads_basic_authorization_header() {
        let ((_, secrets), preview) = parse_curl(
            "curl https://example.test --header='authorization: basic dXNlcjpwYXNzOndvcmQ='",
        )
        .unwrap();
        assert_eq!(preview.authentication, "basic");
        assert_eq!(secrets.get("username"), Some("user"));
        assert_eq!(secrets.get("password"), Some("pass:word"));
    }

    #[test]
    fn rejects_ambiguous_or_unsupported_commands_without_echoing_input() {
        for (command, expected) in [
            ("curl https://example.test -u 'user:secret", INVALID),
            ("curl https://example.test -u 'user:'", INVALID),
            (
                "curl https://example.test -u user:secret https://other.test",
                INVALID,
            ),
            (
                "curl https://example.test -u user:secret -H 'Authorization: Bearer duplicate'",
                INVALID,
            ),
            ("curl https://example.test\n-u user:secret", INVALID),
            ("curl http://example.test -u user:secret", UNSUPPORTED),
            (
                "curl https://user:secret@example.test -u user:secret",
                UNSUPPORTED,
            ),
            ("curl https://@example.test -u user:secret", UNSUPPORTED),
            (
                "curl 'https://example.test/?secret=query' -u user:secret",
                UNSUPPORTED,
            ),
            (
                "curl 'https://example.test/#secret' -u user:secret",
                UNSUPPORTED,
            ),
            (
                "curl https://example.test -H \"Authorization: Bearer $TOKEN\"",
                UNSUPPORTED,
            ),
            (
                "curl https://example.test -u user:secret;other",
                UNSUPPORTED,
            ),
            (
                "curl https://example.test -u user:secret --data @file",
                UNSUPPORTED,
            ),
            (
                "curl https://example.test -u user:secret --config file",
                UNSUPPORTED,
            ),
        ] {
            assert_eq!(parse_curl(command).err().unwrap().to_string(), expected);
        }
    }
}
