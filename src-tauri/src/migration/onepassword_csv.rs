use super::{Entry, Preview, blank, custom, field, finish, login_uri, onepassword::label};
use crate::{browser::current_totp, passkey::origin};
use anyhow::{Result, bail, ensure};
use csv::StringRecord;

#[derive(Clone, Copy)]
enum Column {
    Title,
    Website,
    Username,
    Password,
    Otp,
    Favorite,
    Archived,
    Tags,
    Notes,
}

impl Column {
    fn from_header(header: &str) -> Option<Self> {
        Some(match header.trim().to_ascii_lowercase().as_str() {
            "title" => Self::Title,
            "url" | "website" => Self::Website,
            "username" => Self::Username,
            "password" => Self::Password,
            "otpauth" | "one-time password" => Self::Otp,
            "favorite" | "favorite status" => Self::Favorite,
            "archived" | "archived status" => Self::Archived,
            "tags" => Self::Tags,
            "notes" => Self::Notes,
            _ => return None,
        })
    }
}

struct Columns([Option<usize>; Column::Notes as usize + 1]);

impl Columns {
    fn read(headers: &StringRecord) -> Result<Self> {
        let mut columns = Self([None; Column::Notes as usize + 1]);
        for (index, header) in headers.iter().enumerate() {
            if let Some(column) = Column::from_header(header) {
                ensure!(
                    columns.0[column as usize].replace(index).is_none(),
                    "1Password CSV contains duplicate columns"
                );
            }
        }
        ensure!(
            [
                Column::Title,
                Column::Website,
                Column::Username,
                Column::Password
            ]
            .iter()
            .all(|column| columns.0[*column as usize].is_some()),
            "1Password CSV requires title, website, username and password columns"
        );
        Ok(columns)
    }

    fn value<'a>(&self, row: &'a StringRecord, column: Column) -> &'a str {
        self.0[column as usize]
            .and_then(|index| row.get(index))
            .unwrap_or_default()
    }
}

fn status(value: &str) -> Result<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "1" => Ok(true),
        "false" | "0" | "" => Ok(false),
        _ => bail!("Invalid 1Password CSV status"),
    }
}

pub(super) fn parse(bytes: &[u8], preview: &mut Preview) -> Result<Vec<Entry>> {
    let mut reader = csv::ReaderBuilder::new().from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| anyhow::anyhow!("Invalid 1Password CSV header"))?;
    let columns = Columns::read(headers)?;
    let mut entries = Vec::new();
    for (index, row) in reader.records().enumerate() {
        ensure!(index < 50_000, "Import contains more than 50,000 items");
        let record = index + 1;
        let row = row.map_err(|_| anyhow::anyhow!("Invalid 1Password CSV record {record}"))?;
        let value = |column| columns.value(&row, column);
        if status(value(Column::Archived)).map_err(|_| {
            anyhow::anyhow!("Invalid archive status in 1Password CSV record {record}")
        })? {
            preview.skipped_archived += 1;
            continue;
        }
        let favorite = status(value(Column::Favorite)).map_err(|_| {
            anyhow::anyhow!("Invalid favorite status in 1Password CSV record {record}")
        })?;
        let title = value(Column::Title).trim();
        let website = value(Column::Website).trim();
        let username = value(Column::Username);
        let password = value(Column::Password);
        let notes = value(Column::Notes);
        let tags = value(Column::Tags);
        let otp = value(Column::Otp);
        if [title, website, username, password, notes, tags, otp]
            .iter()
            .all(|value| value.is_empty())
        {
            continue;
        }

        let web_origin = origin(website).ok();
        let name = if title.is_empty() {
            web_origin
                .as_ref()
                .and_then(url::Url::host_str)
                .unwrap_or_default()
        } else {
            title
        };
        let mut entry = blank("login", &label(name, 80))?;
        entry.0.favorite = favorite;
        field(&mut entry, "username", username)?;
        field(&mut entry, "password", password)?;
        field(&mut entry, "secureNote", notes)?;
        if !website.is_empty() {
            if web_origin.is_some() {
                entry
                    .0
                    .login_uris
                    .push(login_uri(website, "origin", preview));
            } else {
                custom(&mut entry, "1Password URL", website)?;
                preview.warn("onepasswordUrls");
            }
        }
        if !otp.is_empty() {
            if current_totp(otp).is_ok() {
                field(&mut entry, "totp", otp)?;
            } else {
                custom(&mut entry, "1Password OTP", otp)?;
                preview.warn("unsupportedTotp");
            }
        }
        if !tags.is_empty() {
            custom(&mut entry, "1Password tags", tags)?;
        }
        finish(&mut entry)?;
        entries.push(entry);
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use crate::{browser::current_totp, login_uri, migration, vault::Vault};
    use std::fs;

    const EXPORT: &str = concat!(
        "\u{feff}Title,Url,Username,Password,OTPAuth,Favorite,Archived,Tags,Notes\r\n",
        "Imported login,https://example.test:8443/login,fixture-user,\" first-fixture-password \",",
        "otpauth://totp/Example?secret=JBSWY3DPEHPK3PXP,TRUE,false,\"red,blue\",\"  first \"\"line\"\"\r\nsecond line  \"\r\n",
        "Archived login,https://archive.example,archive-fixture-user,archive-fixture-password,,false,1,,\r\n",
        "Unsafe URL,https://someone:fixture-uri-secret@example.test,other-fixture-user,other-fixture-password,invalid-fixture-otp-01,0,0,,\r\n",
    );

    #[test]
    fn imports_desktop_csv_fields_and_preserves_origin_isolation() {
        let parsed = migration::parse(EXPORT.as_bytes(), "1password-csv").unwrap();
        assert_eq!(parsed.preview.total, 2);
        assert_eq!(parsed.preview.skipped_archived, 1);
        assert_eq!(parsed.preview.types.get("login"), Some(&2));
        assert!(
            parsed
                .preview
                .warnings
                .iter()
                .any(|warning| warning == "unsupportedTotp")
        );
        assert!(
            parsed
                .preview
                .warnings
                .iter()
                .any(|warning| warning == "onepasswordUrls")
        );
        let (login, secrets) = &parsed.entries[0];
        assert_eq!(login.name, "Imported login");
        assert!(login.favorite);
        assert_eq!(secrets.get("username"), Some("fixture-user"));
        assert_eq!(secrets.get("password"), Some(" first-fixture-password "));
        assert_eq!(
            secrets.get("secureNote"),
            Some("  first \"line\"\r\nsecond line  ")
        );
        assert_eq!(secrets.get("Imported · 1Password tags"), Some("red,blue"));
        assert_eq!(current_totp(secrets.get("totp").unwrap()).unwrap().len(), 6);
        let rule = &login.login_uris[0];
        assert!(login_uri::matches(rule, "https://example.test:8443/other"));
        assert!(!login_uri::matches(rule, "https://example.test/login"));
        assert!(!login_uri::matches(
            rule,
            "https://sub.example.test:8443/login"
        ));
        let (unfillable, secrets) = &parsed.entries[1];
        assert!(unfillable.login_uris.is_empty());
        assert!(
            unfillable
                .modules
                .iter()
                .all(|module| module.value.is_empty())
        );
        assert_eq!(
            secrets.get("Imported · 1Password URL"),
            Some("https://someone:fixture-uri-secret@example.test")
        );
        assert_eq!(
            secrets.get("Imported · 1Password OTP"),
            Some("invalid-fixture-otp-01")
        );
    }

    #[test]
    fn cancel_leaves_the_vault_unchanged_and_conflicts_are_retained() {
        let directory = tempfile::tempdir().unwrap();
        let vault = Vault::open(directory.path().to_path_buf()).unwrap();
        let path = directory.path().join("vault.json");
        let before = fs::read(&path).unwrap();
        let original = migration::parse(EXPORT.as_bytes(), "1password-csv").unwrap();
        let private_values = [
            "first-fixture-password",
            "second-fixture-password",
            "fixture-user",
            "first \"line\"",
            "JBSWY",
            "fixture-uri-secret",
            "invalid-fixture-otp-01",
        ];
        let preview = serde_json::to_string(&original.preview).unwrap();
        assert!(private_values.iter().all(|text| !preview.contains(text)));
        let staged = vault.protect_local_state(&original.entries).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(staged);
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(vault.merge_connections(original.entries).unwrap().added, 2);

        let changed_csv = EXPORT.replace("first-fixture-password", "second-fixture-password");
        let changed = migration::parse(changed_csv.as_bytes(), "1password-csv").unwrap();
        let summary = vault.merge_connections(changed.entries).unwrap();
        assert_eq!((summary.added, summary.merged), (1, 1));
        let entries = vault.list_decrypted_connections().unwrap();
        let first = entries
            .iter()
            .find(|entry| entry.stored.name == "Imported login")
            .unwrap();
        let second = entries
            .iter()
            .find(|entry| entry.secrets.get("password") == Some(" second-fixture-password "))
            .unwrap();
        assert_eq!(
            first.secrets.get("password"),
            Some(" first-fixture-password ")
        );
        assert_ne!(first.stored.name, second.stored.name);
        let repeat = migration::parse(changed_csv.as_bytes(), "1password-csv").unwrap();
        assert_eq!(vault.merge_connections(repeat.entries).unwrap().merged, 2);
        let disk = fs::read_to_string(path).unwrap();
        assert!(private_values.iter().all(|text| !disk.contains(text)));
    }

    #[test]
    fn accepts_documented_aliases_and_rejects_ambiguous_or_invalid_records() {
        let supported = b"Title,Website,Username,Password,One-time password,Favorite status,Archived status\nAlias,https://example.test,u,p,JBSWY3DPEHPK3PXP,true,false\n";
        let parsed = migration::parse(supported, "1password-csv").unwrap();
        assert_eq!(parsed.entries[0].1.get("totp"), Some("JBSWY3DPEHPK3PXP"));
        assert!(parsed.entries[0].0.favorite);
        for invalid in [
            b"Title,URL,Website,Username,Password\n".as_slice(),
            b"Title,URL,Username,Password,OTPAuth,One-time password\n",
            b"Title,URL,Username\n",
            b"Title,URL,Username,Password\nName,https://example.test,fixture-redacted\n",
            b"Title,URL,Username,Password,Archived\nName,https://example.test,user,p,fixture-redacted\n",
            b"Title,URL,Username,Password,Favorite\nName,https://example.test,user,p,fixture-redacted\n",
            b"Title,URL,Username,Password\nName,https://example.test,user,fixture-redacted\xff\n",
        ] {
            let Err(error) = migration::parse(invalid, "1password-csv") else {
                panic!("Invalid CSV must be rejected");
            };
            assert!(!format!("{error:#}").contains("fixture-redacted"));
        }
    }
}
