use super::{Entry, MAX_BYTES, Preview, blank, custom, field, finish, login_uri};
use anyhow::{Result, ensure};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashSet,
    io::{Cursor, Read},
};
use zeroize::{Zeroize, Zeroizing};
use zip::ZipArchive;

const INVALID: &str = "Invalid 1Password export archive";
const MAX_ATTRIBUTES: u64 = 64 * 1024;
const MAX_FILES: usize = 50_002;
const MAX_ITEMS: usize = 50_000;
const MAX_ITEM_FIELDS: usize = 128;
type SecretText = Zeroizing<String>;

#[derive(Deserialize)]
struct Attributes {
    version: u32,
}

#[derive(Deserialize)]
struct Export {
    accounts: Vec<Account>,
}

#[derive(Deserialize)]
struct Account {
    vaults: Option<Vec<Vault>>,
}

#[derive(Deserialize)]
struct Vault {
    attrs: VaultAttributes,
    items: Option<Vec<Item>>,
}

#[derive(Deserialize)]
struct VaultAttributes {
    name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    state: String,
    category_uuid: String,
    fav_index: Option<i64>,
    overview: Option<Overview>,
    details: Option<Details>,
}

#[derive(Default, Deserialize)]
struct Overview {
    title: Option<String>,
    url: Option<SecretText>,
    urls: Option<Vec<Option<Website>>>,
    tags: Option<Vec<SecretText>>,
}

#[derive(Deserialize)]
struct Website {
    url: SecretText,
    mode: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Details {
    login_fields: Option<Vec<Option<LoginField>>>,
    notes_plain: Option<SecretText>,
    password: Option<SecretText>,
    sections: Option<Vec<Option<Section>>>,
    password_history: Option<Vec<serde::de::IgnoredAny>>,
}

#[derive(Deserialize)]
struct LoginField {
    name: Option<String>,
    value: Option<SecretText>,
    designation: Option<String>,
}

#[derive(Deserialize)]
struct Section {
    fields: Option<Vec<Option<SectionField>>>,
}

#[derive(Deserialize)]
struct SectionField {
    id: Option<String>,
    title: Option<String>,
    value: Option<FieldValue>,
}

#[derive(Deserialize)]
#[serde(transparent)]
struct FieldValue(Value);

impl Drop for FieldValue {
    fn drop(&mut self) {
        fn clear(value: &mut Value) {
            match value {
                Value::String(text) => text.zeroize(),
                Value::Array(items) => items.iter_mut().for_each(clear),
                Value::Object(items) => items.values_mut().for_each(clear),
                _ => {}
            }
        }
        clear(&mut self.0);
    }
}

fn archive_member(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    name: &[u8],
    limit: u64,
) -> Result<Zeroizing<Vec<u8>>> {
    let index = archive
        .index_for_name_raw(name)
        .ok_or_else(|| anyhow::anyhow!(INVALID))?;
    let file = archive
        .by_index(index)
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    ensure!(file.is_file(), INVALID);
    let expected = file.size();
    ensure!(
        expected <= limit,
        "1Password export data exceeds the import limit"
    );
    let mut output = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut output)
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    ensure!(output.len() as u64 == expected, INVALID);
    Ok(output)
}

pub(super) fn parse(bytes: &[u8], preview: &mut Preview) -> Result<Vec<Entry>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| anyhow::anyhow!(INVALID))?;
    ensure!(
        archive.len() <= MAX_FILES,
        "1Password export contains too many files"
    );
    archive
        .validate_header_consistency()
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    let attributes = archive_member(&mut archive, b"export.attributes", MAX_ATTRIBUTES)?;
    let attributes: Attributes = serde_json::from_slice(&attributes)
        .map_err(|_| anyhow::anyhow!("Invalid 1Password export attributes"))?;
    ensure!(
        attributes.version == 3,
        "Unsupported 1Password export version"
    );

    preview.skipped_attachments = archive
        .file_names_raw()
        .filter(|name| name.starts_with(b"files/") && !name.ends_with(b"/"))
        .count();
    let json = archive_member(&mut archive, b"export.data", MAX_BYTES)?;
    let export: Export = serde_json::from_slice(&json)
        .map_err(|_| anyhow::anyhow!("Invalid 1Password export data"))?;
    drop(json);

    let mut entries = Vec::new();
    let mut visited = 0;
    for account in export.accounts {
        for vault in account.vaults.into_iter().flatten() {
            let name = label(vault.attrs.name.as_deref().unwrap_or_default().trim(), 120);
            let source = if name.is_empty() { "1Password" } else { &name };
            for item in vault.items.into_iter().flatten() {
                visited += 1;
                ensure!(
                    visited <= MAX_ITEMS,
                    "Import contains more than 50,000 items"
                );
                if item.state == "archived" {
                    preview.skipped_archived += 1;
                    continue;
                }
                let kind = match (item.state.as_str(), item.category_uuid.as_str()) {
                    ("active", "001" | "005") => "login",
                    ("active", "003") => "secureNote",
                    _ => {
                        preview.skipped_types += 1;
                        continue;
                    }
                };
                entries.push(item.into_entry(kind, source, preview)?);
                *preview.sources.entry(source.to_owned()).or_default() += 1;
            }
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{login_uri, migration, vault::Vault};
    use serde_json::json;
    use std::{fs, io::Write};
    use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

    fn pack(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in files {
            zip.start_file(
                *name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    fn make_export(password: &str) -> Vec<u8> {
        let data = json!({"accounts":[{"vaults":[{"attrs":{"name":"Team Fixture"},"items":[
            {"state":"active","categoryUuid":"001","favIndex":1,"overview":{
                "title":"Imported login","url":"https://primary.test/","urls":[
                    {"url":"https://primary.test/","mode":"host"}, {"url":"https://secondary.test/signin"},
                    {"url":"https://blocked.test/","mode":"never"}, {"url":"https://blocked.test/login"},
                    {"url":"https://user:fixture-url-secret@example.test/"}
                ],"tags":["secret-tag-fixture"]},"details":{
                "loginFields":[null,{"designation":"username","value":"fixture-user"},{"designation":"password","value":password},{"name":"Form PIN","value":"fixture-pin"}],
                "notesPlain":"fixture-notes","passwordHistory":[{"value":"obsolete-secret"}],
                "sections":[{"fields":[null,{"id":"TOTP_1","value":{"totp":"otpauth://totp/Fixture?secret=JBSWY3DPEHPK3PXP&digits=8"}},
                    {"title":"Private token","value":{"concealed":"fixture-custom-secret"}},
                    {"title":"Site reference","value":{"reference":"fixture-link"}}]}]
            }},
            {"state":"archived","categoryUuid":"001","overview":{"title":"Archived"}},
            {"state":"active","categoryUuid":"006","overview":{"title":"Document"}},
            {"state":"active","categoryUuid":"003","overview":{"title":"Note"},"details":{"notesPlain":"fixture-note-only","sections":null}}
        ]},{"attrs":{"name":"Personal"},"items":[{"state":"active","categoryUuid":"005","overview":{"title":"Password"},"details":{"password":"fixture-password-only","loginFields":null}}]}]}]});
        pack(&[
            ("export.attributes", br#"{"version":3}"#),
            ("export.data", data.to_string().as_bytes()),
            ("files/fixture-document", b"ignored fixture attachment"),
        ])
    }

    #[test]
    fn projects_documented_fields_and_keeps_metadata_safe() {
        let parsed = migration::parse(&make_export("fixture-password"), "1pux").unwrap();
        assert_eq!(parsed.preview.total, 3);
        assert_eq!(
            parsed.preview.sources,
            [("Personal".into(), 1), ("Team Fixture".into(), 2)].into()
        );
        assert_eq!(parsed.preview.skipped_archived, 1);
        assert_eq!(parsed.preview.skipped_types, 1);
        assert_eq!(parsed.preview.skipped_attachments, 1);
        let metadata = serde_json::to_string(&parsed.preview).unwrap();
        for private in [
            "fixture-user",
            "fixture-password",
            "fixture-url-secret",
            "primary.test",
            "Imported login",
            "JBSWY",
            "fixture-notes",
            "secret-tag-fixture",
        ] {
            assert!(!metadata.contains(private));
        }
        let (login, secrets) = &parsed.entries[0];
        assert_eq!(login.folder, "Team Fixture");
        assert!(login.favorite);
        assert_eq!(secrets.get("username"), Some("fixture-user"));
        assert_eq!(secrets.get("password"), Some("fixture-password"));
        assert_eq!(secrets.get("secureNote"), Some("fixture-notes"));
        assert_eq!(
            crate::browser::current_totp(secrets.get("totp").unwrap())
                .unwrap()
                .len(),
            8
        );
        assert_eq!(
            secrets.get("Imported · Private token"),
            Some("fixture-custom-secret")
        );
        assert_eq!(secrets.get("Imported · Form PIN"), Some("fixture-pin"));
        assert_eq!(
            secrets.get("Imported · 1Password URL"),
            Some("https://user:fixture-url-secret@example.test/")
        );
        assert_eq!(login.login_uris.len(), 4);
        let matches = |target| {
            login
                .login_uris
                .iter()
                .any(|rule| login_uri::matches(rule, target))
        };
        assert!(matches("https://secondary.test/other"));
        assert!(!matches("https://secondary.test:8443/"));
        assert!(!matches("https://blocked.test/anywhere"));
        assert_eq!(parsed.entries[1].0.item_type, "secureNote");
        assert_eq!(
            parsed.entries[2].1.get("password"),
            Some("fixture-password-only")
        );
    }

    #[test]
    fn cancel_and_conflict_merge_keep_vault_data_safe() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().into()).unwrap();
        let path = directory.path().join("vault.json");
        let before = fs::read(&path).unwrap();
        let original = migration::parse(&make_export("first-secret"), "1pux").unwrap();
        let encrypted = vault.protect_local_state(&original.entries).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(encrypted);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(vault.merge_connections(original.entries).unwrap().added, 3);
        let changed = migration::parse(&make_export("second-secret"), "1pux").unwrap();
        let summary = vault.merge_connections(changed.entries).unwrap();
        assert_eq!((summary.added, summary.merged), (1, 2));
        let items = vault.list_decrypted_connections().unwrap();
        let first = items
            .iter()
            .find(|item| item.stored.name == "Imported login")
            .unwrap();
        assert_eq!(first.secrets.get("password"), Some("first-secret"));
        let second = items
            .iter()
            .find(|item| item.secrets.get("password") == Some("second-secret"))
            .unwrap();
        assert_ne!(second.stored.name, first.stored.name);
        let repeat = migration::parse(&make_export("second-secret"), "1pux").unwrap();
        assert_eq!(vault.merge_connections(repeat.entries).unwrap().merged, 3);
        let disk = fs::read_to_string(path).unwrap();
        for private in [
            "first-secret",
            "second-secret",
            "fixture-user",
            "fixture-notes",
            "JBSWY",
            "fixture-custom-secret",
            "fixture-url-secret",
        ] {
            assert!(!disk.contains(private));
        }
    }

    #[test]
    fn rejects_malformed_duplicate_oversized_or_unsupported_archives() {
        let attributes = br#"{"version":3}"#;
        let data = br#"{"accounts":[]}"#;
        let mut duplicate = pack(&[
            ("export.attributes", attributes),
            ("export.data", data),
            ("export.fake", data),
        ]);
        let locations = duplicate
            .windows(b"export.fake".len())
            .enumerate()
            .filter_map(|(index, window)| (window == b"export.fake").then_some(index))
            .collect::<Vec<_>>();
        assert_eq!(locations.len(), 2);
        for index in locations {
            duplicate[index..index + b"export.data".len()].copy_from_slice(b"export.data");
        }
        let truncated = make_export("malformed-fixture-secret");
        assert!(migration::parse(&truncated[..truncated.len() - 16], "1pux").is_err());
        for bytes in [
            b"not a ZIP".to_vec(),
            duplicate,
            pack(&[
                ("export.attributes", br#"{"version":99}"#),
                ("export.data", data),
            ]),
            pack(&[
                ("wrong/export.attributes", attributes),
                ("export.data", data),
            ]),
            pack(&[
                ("export.attributes", attributes),
                ("export.data", b"malformed-fixture-secret"),
            ]),
            pack(&[
                (
                    "export.attributes",
                    &vec![b' '; MAX_ATTRIBUTES as usize + 1],
                ),
                ("export.data", data),
            ]),
        ] {
            let error = migration::parse(&bytes, "1pux")
                .err()
                .expect("invalid archive accepted");
            assert!(!error.to_string().contains("malformed-fixture-secret"));
        }
    }
}

pub(super) fn label(value: &str, max: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max)
        .collect()
}

fn clear_text(value: Option<&SecretText>) -> &str {
    value.map_or("", |value| value.as_str())
}

impl Item {
    fn into_entry(self, kind: &str, source: &str, preview: &mut Preview) -> Result<Entry> {
        let overview = self.overview.unwrap_or_default();
        let details = self.details.unwrap_or_default();
        let name = label(overview.title.as_deref().unwrap_or_default().trim(), 80);
        let mut entry = blank(kind, &name)?;
        entry.0.folder = source.to_owned();
        entry.0.favorite = self.fav_index.is_some_and(|index| index > 0);

        field(
            &mut entry,
            "secureNote",
            clear_text(details.notes_plain.as_ref()),
        )?;
        let login_fields = details.login_fields.as_deref().unwrap_or_default();
        let mut username = false;
        let password = clear_text(details.password.as_ref());
        let mut password_saved = self.category_uuid == "005" && !password.is_empty();
        if password_saved {
            field(&mut entry, "password", password)?;
        }
        ensure!(
            login_fields.len() <= MAX_ITEM_FIELDS,
            "1Password item has too many fields"
        );
        for login in login_fields.iter().flatten() {
            let value = clear_text(login.value.as_ref());
            if value.is_empty() {
                continue;
            }
            match (kind, login.designation.as_deref()) {
                ("login", Some("username")) if !username => {
                    field(&mut entry, "username", value)?;
                    username = true;
                }
                ("login", Some("password")) if !password_saved => {
                    field(&mut entry, "password", value)?;
                    password_saved = true;
                }
                ("login", Some("password")) if value == password => {}
                _ => custom(
                    &mut entry,
                    &label(login.name.as_deref().unwrap_or_default(), 48),
                    value,
                )?,
            }
        }
        if self.category_uuid != "005" {
            custom_if_present(&mut entry, "1Password password", password)?;
        }
        let sections = details.sections.as_deref().unwrap_or_default();
        ensure!(
            sections.len() <= MAX_ITEM_FIELDS,
            "1Password item has too many sections"
        );
        let fields = sections
            .iter()
            .flatten()
            .flat_map(|section| section.fields.as_deref().unwrap_or_default())
            .flatten()
            .collect::<Vec<_>>();
        ensure!(
            login_fields.len() + fields.len() <= MAX_ITEM_FIELDS,
            "1Password item has too many fields"
        );
        let totp_values = fields
            .iter()
            .filter_map(|field| field.totp())
            .collect::<HashSet<_>>();
        let totp = if totp_values.len() == 1 {
            totp_values
                .iter()
                .copied()
                .next()
                .filter(|value| crate::browser::current_totp(value).is_ok())
        } else {
            None
        };
        if let Some(value) = totp {
            field(&mut entry, "totp", value)?;
        } else if !totp_values.is_empty() {
            preview.warn("unsupportedTotp");
        }
        for source_field in fields {
            if totp.is_some() && source_field.totp() == totp {
                continue;
            }
            source_field.save_custom(&mut entry, preview)?;
        }
        if details
            .password_history
            .is_some_and(|history| !history.is_empty())
        {
            preview.warn("onepasswordHistory");
        }
        if let Some(tags) = &overview.tags
            && !tags.is_empty()
        {
            let tags = Zeroizing::new(
                serde_json::to_string(tags)
                    .map_err(|_| anyhow::anyhow!("Invalid 1Password tags"))?,
            );
            custom(&mut entry, "1Password tags", &tags)?;
        }
        overview.save_websites(&mut entry, kind, preview)?;
        finish(&mut entry)?;
        Ok(entry)
    }
}

fn custom_if_present(entry: &mut Entry, name: &str, value: &str) -> Result<()> {
    if !value.is_empty() {
        custom(entry, name, value)?;
    }
    Ok(())
}

impl SectionField {
    fn totp(&self) -> Option<&str> {
        let value = &self.value.as_ref()?.0;
        let seed = value.get("totp").and_then(Value::as_str).or_else(|| {
            self.id.as_deref().filter(|id| id.starts_with("TOTP_"))?;
            value
                .get("string")
                .or_else(|| value.get("concealed"))?
                .as_str()
        })?;
        (!seed.trim().is_empty()).then_some(seed)
    }

    fn save_custom(&self, entry: &mut Entry, preview: &mut Preview) -> Result<()> {
        let Some(value) = self.value.as_ref().map(|value| &value.0) else {
            return Ok(());
        };
        if value.is_null() {
            return Ok(());
        }
        if value.get("reference").is_some() {
            preview.warn("onepasswordFields");
            return Ok(());
        }
        let name = label(self.title.as_deref().unwrap_or_default().trim(), 48);
        let simple = value.as_str().or_else(|| {
            let tagged = value.as_object().filter(|fields| fields.len() == 1)?;
            tagged.values().next()?.as_str()
        });
        if let Some(value) = simple {
            return custom_if_present(entry, &name, value);
        }
        let serialized = Zeroizing::new(
            serde_json::to_string(value)
                .map_err(|_| anyhow::anyhow!("Invalid 1Password custom field"))?,
        );
        custom(entry, &name, &serialized)
    }
}

impl Overview {
    fn save_websites(&self, entry: &mut Entry, kind: &str, preview: &mut Preview) -> Result<()> {
        let urls = self.urls.as_deref().unwrap_or_default();
        ensure!(urls.len() < 128, "1Password item has too many websites");
        let mut urls: Vec<_> = urls
            .iter()
            .flatten()
            .map(|website| {
                (
                    website.url.trim(),
                    website.mode.as_deref().unwrap_or("default"),
                )
            })
            .collect();
        urls.push((clear_text(self.url.as_ref()).trim(), "default"));
        let blocked_origins: HashSet<_> = urls
            .iter()
            .filter(|(_, mode)| !matches!(*mode, "default" | "host"))
            .filter_map(|(value, _)| crate::passkey::origin(value).ok().map(|url| url.origin()))
            .collect();
        let mut seen = HashSet::new();
        for (value, _) in urls {
            if value.is_empty() || !seen.insert(value) {
                continue;
            }
            match crate::passkey::origin(value) {
                Ok(url) if kind == "login" && !blocked_origins.contains(&url.origin()) => {
                    entry.0.login_uris.push(login_uri(value, "origin", preview));
                }
                Ok(_) if kind == "login" => {
                    entry.0.login_uris.push(login_uri(value, "never", preview));
                    preview.warn("onepasswordFillRules");
                }
                _ => {
                    custom(entry, "1Password URL", value)?;
                    preview.warn("onepasswordUrls");
                }
            }
        }
        Ok(())
    }
}
