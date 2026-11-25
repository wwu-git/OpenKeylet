use super::{Entry, Preview, blank, custom, field, finish, login_uri};
use anyhow::{Context, Result, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use quick_xml::{
    Reader, XmlVersion,
    escape::resolve_predefined_entity,
    events::{BytesStart, Event},
};
use std::collections::HashMap;
use zeroize::Zeroizing;

const INVALID: &str = "Invalid KeePass 2.x XML export";
const MAX_DEPTH: usize = 64;
const MAX_ITEMS: usize = 50_000;
const MAX_VALUE: usize = 1024 * 1024;
type XmlText = Zeroizing<String>;

#[derive(Default)]
struct Group {
    parent: Option<usize>,
    name: Option<XmlText>,
    uuid: Option<[u8; 16]>,
}

struct CurrentEntry {
    group: usize,
    fields: HashMap<Key, XmlText>,
    protected_native_totp: bool,
}

#[derive(Default)]
struct StringField {
    key: Option<XmlText>,
    value: Option<XmlText>,
    protected: bool,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Key {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Otp,
    TimeSecret,
    TimePeriod,
    TimeLength,
    TimeAlgorithm,
}

impl Key {
    fn parse(key: &str) -> Option<Self> {
        Some(match key.to_ascii_lowercase().as_str() {
            "title" => Self::Title,
            "username" => Self::Username,
            "password" => Self::Password,
            "url" => Self::Url,
            "notes" => Self::Notes,
            "otp" | "totp" | "totp seed" => Self::Otp,
            "timeotp-secret-base32" => Self::TimeSecret,
            "timeotp-period" => Self::TimePeriod,
            "timeotp-length" => Self::TimeLength,
            "timeotp-algorithm" => Self::TimeAlgorithm,
            _ => return None,
        })
    }
}

enum TextSlot {
    GroupName,
    GroupUuid,
    RecycleUuid,
    RecycleEnabled,
    Key,
    Value,
}

enum Frame {
    File,
    Meta,
    Root,
    Group(usize),
    Entry(CurrentEntry),
    Field(StringField),
    Text(TextSlot, XmlText),
    History,
    Ignore,
}

#[derive(Default)]
struct Export {
    frames: Vec<Frame>,
    groups: Vec<Group>,
    entries: Vec<CurrentEntry>,
    file_seen: bool,
    root_seen: bool,
    recycle_uuid: Option<[u8; 16]>,
    recycle_enabled: Option<bool>,
}

impl Export {
    fn start(&mut self, tag: BytesStart<'_>, preview: &mut Preview) -> Result<()> {
        ensure!(
            self.frames.len() < MAX_DEPTH,
            "KeePass XML nesting is too deep"
        );
        let mut protected = false;
        for attribute in tag.attributes() {
            let attribute = attribute.map_err(|_| anyhow::anyhow!(INVALID))?;
            let value = attribute
                .normalized_value(XmlVersion::Implicit1_0)
                .map_err(|_| anyhow::anyhow!(INVALID))?;
            ensure!(value.chars().all(valid_xml_char), INVALID);
            if (attribute.key.as_ref() == b"Protected" && !value.eq_ignore_ascii_case("false"))
                || attribute.key.as_ref() == b"Ref"
            {
                protected = true;
            }
        }
        let text = |slot| Frame::Text(slot, XmlText::default());
        let name = tag.name();
        let frame = match (self.frames.last_mut(), name.as_ref()) {
            (None, b"KeePassFile") if !self.file_seen => {
                self.file_seen = true;
                Frame::File
            }
            (None | Some(Frame::Text(..)), _) => bail!(INVALID),
            (Some(Frame::File), b"Root") if !self.root_seen => {
                self.root_seen = true;
                Frame::Root
            }
            (Some(Frame::File), b"Root") => bail!(INVALID),
            (Some(Frame::File), b"Meta") => Frame::Meta,
            (Some(Frame::Meta), b"RecycleBinUUID") => text(TextSlot::RecycleUuid),
            (Some(Frame::Meta), b"RecycleBinEnabled") => text(TextSlot::RecycleEnabled),
            (Some(parent @ (Frame::Root | Frame::Group(_))), b"Group") => {
                let parent = match parent {
                    Frame::Group(index) => Some(*index),
                    _ => None,
                };
                ensure!(
                    self.groups.len() < MAX_ITEMS,
                    "KeePass export has too many groups"
                );
                let index = self.groups.len();
                self.groups.push(Group {
                    parent,
                    ..Default::default()
                });
                Frame::Group(index)
            }
            (Some(Frame::Group(index)), b"Entry") => Frame::Entry(CurrentEntry {
                group: *index,
                fields: HashMap::new(),
                protected_native_totp: false,
            }),
            (Some(Frame::Group(_)), b"Name") => text(TextSlot::GroupName),
            (Some(Frame::Group(_)), b"UUID") => text(TextSlot::GroupUuid),
            (Some(Frame::Entry(_)), b"String") => Frame::Field(StringField::default()),
            (Some(Frame::Entry(_)), b"History") => Frame::History,
            (Some(Frame::Entry(_)), b"Binary") => {
                preview.warn("keepassAttachments");
                Frame::Ignore
            }
            (Some(Frame::History), b"Entry") => {
                preview.warn("keepassHistory");
                Frame::Ignore
            }
            (Some(Frame::Field(_)), b"Key") => text(TextSlot::Key),
            (Some(Frame::Field(field)), b"Value") => {
                field.protected = protected;
                text(TextSlot::Value)
            }
            _ => Frame::Ignore,
        };
        self.frames.push(frame);
        Ok(())
    }

    fn append(&mut self, text: &str) -> Result<()> {
        ensure!(text.chars().all(valid_xml_char), INVALID);
        match self.frames.last_mut() {
            Some(Frame::Text(_, value)) => {
                ensure!(
                    value.len() + text.len() <= MAX_VALUE,
                    "KeePass XML field exceeds 1 MiB"
                );
                value.push_str(text);
            }
            None => ensure!(text.trim().is_empty(), INVALID),
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, preview: &mut Preview) -> Result<()> {
        match (self.frames.pop().context(INVALID)?, self.frames.last_mut()) {
            (Frame::Text(slot, value), parent) => match (slot, parent) {
                (TextSlot::GroupName, Some(Frame::Group(group))) => {
                    ensure!(self.groups[*group].name.replace(value).is_none(), INVALID);
                }
                (TextSlot::GroupUuid, Some(Frame::Group(group))) => {
                    ensure!(
                        self.groups[*group].uuid.replace(uuid(&value)?).is_none(),
                        INVALID
                    );
                }
                (TextSlot::RecycleUuid, Some(Frame::Meta)) => {
                    ensure!(self.recycle_uuid.replace(uuid(&value)?).is_none(), INVALID);
                }
                (TextSlot::RecycleEnabled, Some(Frame::Meta)) => {
                    let enabled = match value.trim() {
                        text if text.eq_ignore_ascii_case("true") => true,
                        text if text.eq_ignore_ascii_case("false") => false,
                        _ => bail!(INVALID),
                    };
                    ensure!(self.recycle_enabled.replace(enabled).is_none(), INVALID);
                }
                (TextSlot::Key, Some(Frame::Field(field))) => {
                    ensure!(field.key.replace(value).is_none(), INVALID);
                }
                (TextSlot::Value, Some(Frame::Field(field))) => {
                    ensure!(field.value.replace(value).is_none(), INVALID);
                }
                _ => bail!(INVALID),
            },
            (Frame::Field(data), Some(Frame::Entry(entry))) => {
                let key = data.key.context(INVALID)?;
                let value = data.value.context(INVALID)?;
                if data.protected {
                    preview.warn("keepassProtected");
                    entry.protected_native_totp |= matches!(
                        Key::parse(&key),
                        Some(
                            Key::TimeSecret
                                | Key::TimePeriod
                                | Key::TimeLength
                                | Key::TimeAlgorithm
                        )
                    );
                } else if let Some(key) = Key::parse(&key) {
                    ensure!(entry.fields.insert(key, value).is_none(), INVALID);
                } else if !value.is_empty() {
                    preview.warn("keepassCustomFields");
                }
            }
            (Frame::Entry(entry), _) => {
                ensure!(
                    self.entries.len() < MAX_ITEMS,
                    "Import contains more than 50,000 items"
                );
                self.entries.push(entry);
            }
            _ => {}
        }
        Ok(())
    }

    fn convert(self, preview: &mut Preview) -> Result<Vec<Entry>> {
        let recycle = self
            .recycle_uuid
            .filter(|uuid| self.recycle_enabled != Some(false) && *uuid != [0; 16]);
        let mut entries = Vec::new();
        for current in self.entries {
            let mut group_index = Some(current.group);
            let mut path = Vec::new();
            let mut deleted = false;
            while let Some(index) = group_index {
                let group = &self.groups[index];
                if recycle.is_some() && group.uuid == recycle {
                    deleted = true;
                    break;
                }
                if let Some(name) = group.name.as_deref().filter(|name| !name.trim().is_empty()) {
                    path.push(name.trim());
                }
                group_index = group.parent;
            }
            if deleted {
                preview.skipped_deleted += 1;
                continue;
            }
            path.reverse();
            let path_length = path.iter().map(|name| name.chars().count()).sum::<usize>()
                + path.len().saturating_sub(1) * 3;
            ensure!(
                path_length <= 120,
                "KeePass group path exceeds 120 characters"
            );
            if let Some(entry) = current.to_entry(&path.join(" / "), preview)? {
                entries.push(entry);
            }
        }
        Ok(entries)
    }
}

fn valid_xml_char(ch: char) -> bool {
    matches!(ch, '\t' | '\n' | '\r' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'..='\u{10ffff}')
}

fn uuid(text: &str) -> Result<[u8; 16]> {
    if text.trim().is_empty() {
        return Ok([0; 16]);
    }
    let bytes = STANDARD
        .decode(text.trim())
        .map_err(|_| anyhow::anyhow!(INVALID))?;
    bytes.try_into().map_err(|_| anyhow::anyhow!(INVALID))
}

pub(super) fn parse(bytes: &[u8], preview: &mut Preview) -> Result<Vec<Entry>> {
    let xml =
        std::str::from_utf8(bytes).map_err(|_| anyhow::anyhow!("KeePass XML must be UTF-8"))?;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().check_comments = true;
    reader.config_mut().expand_empty_elements = true;
    let mut data = Export::default();
    let mut declaration_seen = false;
    loop {
        match reader.read_event().map_err(|_| anyhow::anyhow!(INVALID))? {
            Event::Start(tag) => data.start(tag, preview)?,
            Event::End(_) => data.end(preview)?,
            Event::Text(text) => {
                data.append(&text.xml10_content().map_err(|_| anyhow::anyhow!(INVALID))?)?
            }
            Event::CData(text) => {
                ensure!(!data.frames.is_empty(), INVALID);
                data.append(&text.xml10_content().map_err(|_| anyhow::anyhow!(INVALID))?)?;
            }
            Event::GeneralRef(reference) => {
                ensure!(!data.frames.is_empty(), INVALID);
                if let Some(ch) = reference
                    .resolve_char_ref()
                    .map_err(|_| anyhow::anyhow!(INVALID))?
                {
                    data.append(ch.encode_utf8(&mut [0; 4]))?;
                } else {
                    let name = reference.decode().map_err(|_| anyhow::anyhow!(INVALID))?;
                    data.append(
                        resolve_predefined_entity(&name)
                            .context("Unsupported KeePass XML entity")?,
                    )?;
                }
            }
            Event::Decl(declaration) => {
                ensure!(!data.file_seen && !declaration_seen, INVALID);
                declaration_seen = true;
                ensure!(
                    declaration
                        .version()
                        .map_err(|_| anyhow::anyhow!(INVALID))?
                        .as_ref()
                        == b"1.0",
                    INVALID
                );
                if let Some(encoding) = declaration.encoding() {
                    ensure!(
                        encoding
                            .map_err(|_| anyhow::anyhow!(INVALID))?
                            .eq_ignore_ascii_case(b"UTF-8"),
                        "KeePass XML must be UTF-8"
                    );
                }
            }
            Event::DocType(_) => bail!("KeePass XML document types are unsupported"),
            Event::Eof => break,
            Event::Comment(_) | Event::PI(_) => {}
            Event::Empty(_) => unreachable!(),
        }
    }
    ensure!(
        data.file_seen && data.root_seen && data.frames.is_empty(),
        INVALID
    );
    data.convert(preview)
}

impl CurrentEntry {
    fn get(&self, key: Key) -> &str {
        self.fields.get(&key).map_or("", |value| value.as_str())
    }

    fn native_totp(&self) -> Option<XmlText> {
        let seed = self.get(Key::TimeSecret).trim();
        if seed.is_empty() || self.protected_native_totp {
            return None;
        }
        let mut url = url::Url::parse("otpauth://totp/KeePass").ok()?;
        let mut query = url.query_pairs_mut();
        query.append_pair("secret", seed);
        for (key, name) in [(Key::TimePeriod, "period"), (Key::TimeLength, "digits")] {
            let value = self.get(key).trim();
            if !value.is_empty() {
                query.append_pair(name, value);
            }
        }
        let algorithm = self
            .get(Key::TimeAlgorithm)
            .trim()
            .to_ascii_uppercase()
            .replace('-', "");
        if !algorithm.is_empty() {
            query.append_pair(
                "algorithm",
                algorithm.strip_prefix("HMAC").unwrap_or(&algorithm),
            );
        }
        drop(query);
        let uri = XmlText::new(url.into());
        crate::browser::current_totp(&uri).ok()?;
        Some(uri)
    }

    fn to_entry(&self, folder: &str, preview: &mut Preview) -> Result<Option<Entry>> {
        let title = self.get(Key::Title).trim();
        let url = self.get(Key::Url).trim();
        let (username, password, notes, otp) = (
            self.get(Key::Username),
            self.get(Key::Password),
            self.get(Key::Notes),
            self.get(Key::Otp),
        );
        if self.fields.values().all(|value| value.is_empty()) {
            preview.warn("keepassEmptyEntries");
            return Ok(None);
        }
        let fallback = crate::passkey::origin(url)
            .ok()
            .and_then(|uri| uri.host_str().map(str::to_owned))
            .unwrap_or_else(|| "KeePass entry".to_owned());
        let kind = if url.is_empty()
            && username.is_empty()
            && password.is_empty()
            && otp.is_empty()
            && self.get(Key::TimeSecret).is_empty()
            && !notes.is_empty()
        {
            "secureNote"
        } else {
            "login"
        };
        let name = if title.is_empty() { &fallback } else { title };
        let mut entry = blank(kind, name)?;
        entry.0.folder = folder.to_owned();
        for (name, value) in [
            ("username", username),
            ("password", password),
            ("secureNote", notes),
        ] {
            field(&mut entry, name, value)?;
        }
        if !url.is_empty() {
            if crate::passkey::origin(url).is_ok() {
                entry.0.login_uris.push(login_uri(url, "origin", preview));
            } else {
                custom(&mut entry, "KeePass URL", url)?;
                preview.warn("keepassNonWebUris");
            }
        }
        if let Some(native) = self.native_totp() {
            field(&mut entry, "totp", &native)?;
        } else {
            if !otp.is_empty() {
                field(&mut entry, "totp", otp)?;
                if crate::browser::current_totp(otp).is_err() {
                    preview.warn("unsupportedTotp");
                }
            }
            for (key, name) in [
                (Key::TimeSecret, "TimeOtp-Secret-Base32"),
                (Key::TimePeriod, "TimeOtp-Period"),
                (Key::TimeLength, "TimeOtp-Length"),
                (Key::TimeAlgorithm, "TimeOtp-Algorithm"),
            ] {
                let value = self.get(key);
                if !value.is_empty() {
                    custom(&mut entry, name, value)?;
                    preview.warn("unsupportedTotp");
                }
            }
        }
        finish(&mut entry)?;
        Ok(Some(entry))
    }
}

#[cfg(test)]
mod tests {
    use crate::{browser::current_totp, login_uri, migration, vault::Vault};
    use std::fs;

    const EXPORT: &[u8] = br#"<?xml version="1.0" encoding="utf-8"?>
<KeePassFile>
  <Meta><RecycleBinEnabled>True</RecycleBinEnabled><RecycleBinUUID>AQEBAQEBAQEBAQEBAQEBAQ==</RecycleBinUUID></Meta>
  <Root><Group>
    <Name>Fixtures</Name>
    <Group>
      <Name>Sites &amp; Tools</Name>
      <Entry>
        <String><Key>Title</Key><Value>Test &#65;ccount</Value></String>
        <String><Key>UserName</Key><Value ProtectInMemory="True">fixture&apos;user</Value></String>
        <String><Key>Password</Key><Value> fixture-secret&#x26; </Value></String>
        <String><Key>URL</Key><Value>https://example.test:8443/path</Value></String>
        <String><Key>Notes</Key><Value><![CDATA[fixture note <&>]]></Value></String>
        <String><Key>TimeOtp-Secret-Base32</Key><Value>JBSWY3DPEHPK3PXP</Value></String>
        <String><Key>TimeOtp-Length</Key><Value>8</Value></String>
        <String><Key>TimeOtp-Period</Key><Value>60</Value></String>
        <String><Key>TimeOtp-Algorithm</Key><Value>HMAC-SHA-256</Value></String>
        <Binary><Key>attachment</Key><Value Ref="0"/></Binary>
        <History><Entry><String><Key>Password</Key><Value>old-credential</Value></String></Entry></History>
      </Entry>
    </Group>
    <Group><UUID>AQEBAQEBAQEBAQEBAQEBAQ==</UUID><Name>Different name</Name><Group>
      <Entry><String><Key>Title</Key><Value>Deleted record</Value></String></Entry>
    </Group></Group>
  </Group></Root>
</KeePassFile>"#;

    #[test]
    fn imports_current_fields_and_preserves_totp_parameters() {
        let parsed = migration::parse(EXPORT, "keepass").unwrap();
        assert_eq!(parsed.preview.total, 1);
        assert_eq!(parsed.preview.skipped_deleted, 1);
        assert!(
            parsed
                .preview
                .warnings
                .iter()
                .any(|key| key == "keepassHistory")
        );
        assert!(
            parsed
                .preview
                .warnings
                .iter()
                .any(|key| key == "keepassAttachments")
        );
        let (item, secrets) = &parsed.entries[0];
        assert_eq!(item.name, "Test Account");
        assert_eq!(item.folder, "Fixtures / Sites & Tools");
        assert_eq!(secrets.get("username"), Some("fixture'user"));
        assert_eq!(secrets.get("password"), Some(" fixture-secret& "));
        assert_eq!(secrets.get("secureNote"), Some("fixture note <&>"));
        let otp = secrets.get("totp").unwrap();
        assert!(otp.contains("digits=8"));
        assert!(otp.contains("period=60"));
        assert!(otp.contains("algorithm=SHA256"));
        assert_eq!(current_totp(otp).unwrap().len(), 8);
        assert!(login_uri::matches(
            &item.login_uris[0],
            "https://example.test:8443/other"
        ));
        assert!(!login_uri::matches(
            &item.login_uris[0],
            "https://example.test/other"
        ));

        let export = std::str::from_utf8(EXPORT).unwrap().replace(
            "<RecycleBinEnabled>True</RecycleBinEnabled>",
            "<RecycleBinEnabled>False</RecycleBinEnabled>",
        );
        let parsed = migration::parse(export.as_bytes(), "keepass").unwrap();
        assert_eq!(parsed.preview.total, 2);
        assert_eq!(parsed.preview.skipped_deleted, 0);
    }

    #[test]
    fn preview_and_cancel_do_not_write_and_conflicts_preserve_both_versions() {
        let dir = tempfile::tempdir().unwrap();
        let vault = Vault::open(dir.path().to_path_buf()).unwrap();
        let path = dir.path().join("vault.json");
        let before = fs::read(&path).unwrap();
        let parsed = migration::parse(EXPORT, "keepass").unwrap();
        let metadata = serde_json::to_string(&parsed.preview).unwrap();
        for private in [
            "fixture-secret",
            "fixture note",
            "fixture'user",
            "JBSWY3DPEHPK3PXP",
        ] {
            assert!(!metadata.contains(private));
        }
        drop(vault.protect_local_state(&parsed.entries).unwrap());
        assert_eq!(fs::read(&path).unwrap(), before);
        let staged = vault.protect_local_state(&parsed.entries).unwrap();
        let summary = vault
            .merge_connections(vault.unprotect_local_state(&staged).unwrap())
            .unwrap();
        assert_eq!(summary.added, 1);

        let update = std::str::from_utf8(EXPORT)
            .unwrap()
            .replace("fixture-secret", "other-credential");
        let update = migration::parse(update.as_bytes(), "keepass").unwrap();
        assert_eq!(vault.merge_connections(update.entries).unwrap().added, 1);
        let items = vault.list_decrypted_connections().unwrap();
        assert_eq!(items.len(), 2);
        let original = items
            .iter()
            .find(|entry| entry.stored.name == "Test Account")
            .unwrap();
        assert_eq!(original.secrets.get("password"), Some(" fixture-secret& "));
        let renamed = items
            .iter()
            .find(|entry| entry.stored.name != "Test Account")
            .unwrap();
        assert_eq!(renamed.secrets.get("password"), Some(" other-credential& "));
        let retry = migration::parse(EXPORT, "keepass").unwrap();
        assert_eq!(vault.merge_connections(retry.entries).unwrap().merged, 1);
        assert_eq!(vault.list_connections().unwrap().len(), 2);
        let disk = fs::read_to_string(path).unwrap();
        for private in [
            "fixture-secret",
            "other-credential",
            "fixture note",
            "JBSWY3DPEHPK3PXP",
        ] {
            assert!(!disk.contains(private));
        }
    }

    #[test]
    fn protected_and_unsafe_url_fields_never_become_autofill_metadata() {
        for url in [
            "cmd://fixture-token",
            "https://user:fixture-token@example.test/",
        ] {
            let export = format!(
                r#"<KeePassFile><Root><Group><Entry>
                <String><Key>URL</Key><Value>{url}</Value></String>
                <String><Key>Password</Key><Value Protected="True">encoded-secret</Value></String>
                <String><Key>TimeOtp-Secret-Base32</Key><Value>JBSWY3DPEHPK3PXP</Value></String>
                <String><Key>TimeOtp-Length</Key><Value Protected="True">encoded-parameter</Value></String>
            </Entry></Group></Root></KeePassFile>"#
            );
            let parsed = migration::parse(export.as_bytes(), "keepass").unwrap();
            assert!(
                parsed
                    .preview
                    .warnings
                    .iter()
                    .any(|key| key == "keepassProtected")
            );
            assert!(
                parsed
                    .preview
                    .warnings
                    .iter()
                    .any(|key| key == "keepassNonWebUris")
            );
            assert!(
                parsed
                    .preview
                    .warnings
                    .iter()
                    .any(|key| key == "unsupportedTotp")
            );
            let (item, secrets) = &parsed.entries[0];
            assert!(item.login_uris.is_empty());
            assert!(
                !serde_json::to_string(item)
                    .unwrap()
                    .contains("fixture-token")
            );
            assert_eq!(secrets.get("Imported · KeePass URL"), Some(url));
            assert!(secrets.get("password").is_none());
            assert!(secrets.get("totp").is_none());
        }
    }

    #[test]
    fn rejects_malformed_or_unsafe_xml_without_echoing_input() {
        for export in [
            "<KeePassFile><Root><fixture-secret></Root></KeePassFile>",
            "<KeePassFile><Root>&fixture-secret;</Root></KeePassFile>",
            "<KeePassFile><Root a='&fixture-secret;'/></KeePassFile>",
            "<!DOCTYPE KeePassFile [<!ENTITY s SYSTEM 'file:///fixture-secret'>]><KeePassFile><Root/></KeePassFile>",
            "<KeePassFile><Root/></KeePassFile><fixture-secret/>",
        ] {
            let error = migration::parse(export.as_bytes(), "keepass")
                .err()
                .unwrap();
            assert!(!error.to_string().contains("fixture-secret"));
        }
        let deep = format!(
            "<KeePassFile><Root>{}{}</Root></KeePassFile>",
            "<x>".repeat(64),
            "</x>".repeat(64)
        );
        assert!(migration::parse(deep.as_bytes(), "keepass").is_err());
    }
}
