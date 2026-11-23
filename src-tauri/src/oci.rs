use anyhow::{Result, anyhow, bail, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Method, Request,
    header::{AUTHORIZATION, HeaderValue},
};
use ring::{
    rand::SystemRandom,
    signature::{RSA_PKCS1_SHA256, RsaKeyPair},
};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt::Write, time::Duration};
use url::Url;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub(crate) const AUTH_MODE: &str = "oci-signature";
pub(crate) const INSTANCES_PATH: &str = "/20160918/instances";

// Oracle's public OC1 region identifiers. Other realms use different DNS suffixes.
// https://docs.oracle.com/en-us/iaas/Content/General/Concepts/regions.htm
pub(crate) const REGIONS: &[&str] = &[
    "af-johannesburg-1",
    "af-casablanca-1",
    "ap-batam-1",
    "ap-chuncheon-1",
    "ap-hyderabad-1",
    "ap-kulai-2",
    "ap-melbourne-1",
    "ap-mumbai-1",
    "ap-osaka-1",
    "ap-seoul-1",
    "ap-singapore-1",
    "ap-singapore-2",
    "ap-sydney-1",
    "ap-tokyo-1",
    "ca-montreal-1",
    "ca-toronto-1",
    "eu-amsterdam-1",
    "eu-frankfurt-1",
    "eu-madrid-1",
    "eu-madrid-3",
    "eu-marseille-1",
    "eu-milan-1",
    "eu-paris-1",
    "eu-stockholm-1",
    "eu-turin-1",
    "eu-zurich-1",
    "il-jerusalem-1",
    "me-abudhabi-1",
    "me-dubai-1",
    "me-jeddah-1",
    "me-riyadh-1",
    "mx-monterrey-1",
    "mx-queretaro-1",
    "sa-bogota-1",
    "sa-santiago-1",
    "sa-saopaulo-1",
    "sa-valparaiso-1",
    "sa-vinhedo-1",
    "uk-cardiff-1",
    "uk-london-1",
    "us-ashburn-1",
    "us-chicago-1",
    "us-phoenix-1",
    "us-sanjose-1",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Config {
    pub region: String,
    pub tenancy_ocid: String,
    pub user_ocid: String,
    pub fingerprint: String,
}

impl Config {
    pub fn base_url(&self) -> Result<Url> {
        ensure!(
            REGIONS.contains(&self.region.as_str()),
            "Unsupported OCI commercial region"
        );
        Ok(Url::parse(&format!(
            "https://iaas.{}.oraclecloud.com/20160918/",
            self.region
        ))?)
    }

    pub fn validate(&self) -> Result<()> {
        self.base_url()?;
        for (value, kind) in [
            (&self.tenancy_ocid, "ocid1.tenancy.oc1.."),
            (&self.user_ocid, "ocid1.user.oc1.."),
        ] {
            ensure!(
                value
                    .strip_prefix(kind)
                    .is_some_and(|suffix| !suffix.is_empty()
                        && suffix.len() <= 240
                        && suffix.bytes().all(|b| b.is_ascii_alphanumeric())),
                "Invalid OCI tenancy or user OCID"
            );
        }
        ensure!(
            self.fingerprint.len() == 47
                && self
                    .fingerprint
                    .split(':')
                    .all(|part| part.len() == 2 && part.bytes().all(|b| b.is_ascii_hexdigit())),
            "Invalid OCI API key fingerprint"
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Credential {
    pub config: Config,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    key_pem: Option<String>,
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OciCredential")
            .field("configured", &self.is_configured())
            .finish()
    }
}

impl Credential {
    pub fn new(config: Config, pem: &str) -> Result<Self> {
        config.validate()?;
        validate_pem(pem)?;
        Ok(Self {
            config,
            key_pem: Some(pem.to_owned()),
        })
    }

    pub fn is_configured(&self) -> bool {
        self.key_pem.is_some()
    }

    pub fn forget_key(&mut self) {
        self.key_pem.zeroize();
        self.key_pem = None;
    }

    fn sign(&self, request: &mut Request, date: &str) -> Result<HeaderValue> {
        let key = parse_key(
            self.key_pem
                .as_deref()
                .ok_or_else(|| anyhow!("Reimport the OCI signing key"))?,
        )?;
        let url = request.url();
        let host = url.host_str().ok_or_else(|| anyhow!("Missing OCI host"))?;
        let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let message = format!("(request-target): get {target}\nhost: {host}\nx-date: {date}");
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            message.as_bytes(),
            &mut signature,
        )
        .map_err(|_| anyhow!("OCI request signing failed"))?;
        let mut header = HeaderValue::from_str(&format!(
            "Signature version=\"1\",keyId=\"{}/{}/{}\",algorithm=\"rsa-sha256\",headers=\"(request-target) host x-date\",signature=\"{}\"",
            self.config.tenancy_ocid,
            self.config.user_ocid,
            self.config.fingerprint.to_ascii_lowercase(),
            STANDARD.encode(signature),
        ))?;
        header.set_sensitive(true);
        request
            .headers_mut()
            .insert("x-date", HeaderValue::from_str(date)?);
        request.headers_mut().insert(AUTHORIZATION, header.clone());
        Ok(header)
    }
}

pub(crate) fn validate_pem(pem: &str) -> Result<()> {
    parse_key(pem).map(|_| ())
}

fn parse_key(pem: &str) -> Result<RsaKeyPair> {
    let error =
        || anyhow!("Expected an unencrypted RSA private key in PEM format (at least 2048 bits)");
    ensure!(pem.len() <= 64 * 1024, "OCI signing key file is too large");
    let pem = pem.trim();
    // Oracle's downloadable API keys may include this marker after the PEM.
    let pem = pem.strip_suffix("\nOCI_API_KEY").map_or(pem, str::trim_end);
    for (label, pkcs8) in [("PRIVATE KEY", true), ("RSA PRIVATE KEY", false)] {
        if let Some(data) = pem
            .strip_prefix(&format!("-----BEGIN {label}-----"))
            .and_then(|body| body.strip_suffix(&format!("-----END {label}-----")))
        {
            let data = Zeroizing::new(data.split_ascii_whitespace().collect::<String>());
            let der = Zeroizing::new(STANDARD.decode(data.as_bytes()).map_err(|_| error())?);
            return (if pkcs8 {
                RsaKeyPair::from_pkcs8(&der)
            } else {
                RsaKeyPair::from_der(&der)
            })
            .map_err(|_| error());
        }
    }
    Err(error())
}

pub(crate) fn validate_request(
    config: &Config,
    mut url: Url,
    input: &crate::executor::ApiRequestInput,
) -> Result<Url> {
    let base = config.base_url()?;
    ensure!(
        url.origin() == base.origin()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "OCI request target is outside the configured region"
    );
    ensure!(
        input.method.eq_ignore_ascii_case("GET") && url.path() == INSTANCES_PATH,
        "OCI supports only GET /20160918/instances"
    );
    ensure!(
        input.body.is_none()
            && input.body_base64.is_none()
            && input.form.is_empty()
            && input.files.is_empty(),
        "OCI ListInstances does not accept a request body"
    );
    ensure!(
        input.save_response_to.is_none(),
        "OCI ListInstances returns results directly"
    );
    ensure!(
        !input.headers.keys().any(|key| [
            "host",
            "authorization",
            "date",
            "x-date",
            "content-length",
            "transfer-encoding",
            "proxy-authorization"
        ]
        .iter()
        .any(|blocked| key.eq_ignore_ascii_case(blocked))),
        "OCI request cannot override signing or transport headers"
    );

    let mut seen = HashSet::new();
    let mut query = String::new();
    for (name, value) in url.query_pairs() {
        ensure!(
            seen.insert(name.to_string()),
            "Duplicate OCI query parameter"
        );
        match name.as_ref() {
            "compartmentId" | "page" => {
                ensure!(!value.trim().is_empty(), "Empty OCI query parameter")
            }
            "limit" => ensure!(
                value.parse::<i32>().is_ok_and(|limit| limit > 0)
                    && value.bytes().all(|byte| byte.is_ascii_digit()),
                "Invalid OCI limit"
            ),
            _ => bail!("Unsupported OCI query parameter"),
        }
        if !query.is_empty() {
            query.push('&');
        }
        encode(&name, &mut query);
        query.push('=');
        encode(&value, &mut query);
    }
    ensure!(
        seen.contains("compartmentId"),
        "OCI ListInstances requires compartmentId"
    );
    url.set_query(Some(&query));
    Ok(url)
}

fn encode(value: &str, output: &mut String) {
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            output.push(char::from(byte));
        } else {
            let _ = write!(output, "%{byte:02X}");
        }
    }
}

pub(crate) fn client_builder(timeout: Option<u64>) -> reqwest::ClientBuilder {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(
            timeout.filter(|t| *t > 0).unwrap_or(30),
        ))
}

pub(crate) fn prepare(
    credential: &Credential,
    target: Url,
    input: &crate::executor::ApiRequestInput,
    date: &str,
) -> Result<(Request, HeaderValue)> {
    let target = validate_request(&credential.config, target, input)?;
    credential.config.validate()?;
    let mut request = Request::new(Method::GET, target);
    for (name, value) in &input.headers {
        request.headers_mut().insert(
            reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| anyhow!("Invalid OCI request header"))?,
            HeaderValue::from_str(value).map_err(|_| anyhow!("Invalid OCI request header"))?,
        );
    }
    let signature = credential.sign(&mut request, date)?;
    Ok((request, signature))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::executor::ApiRequestInput;

    pub(crate) const KEY: &str = include_str!("../tests/fixtures/oci/test-key.pem");
    pub(crate) const HOST: &str = "iaas.ap-tokyo-1.oraclecloud.com";
    const DATE: &str = "Thu, 01 Jan 2026 00:00:00 GMT";

    pub(crate) fn config() -> Config {
        Config {
            region: "ap-tokyo-1".into(),
            tenancy_ocid: "ocid1.tenancy.oc1..test".into(),
            user_ocid: "ocid1.user.oc1..test".into(),
            fingerprint: ["00"; 16].join(":"),
        }
    }

    #[test]
    fn oci_signature_matches_independent_rsa_vector() {
        let credential = Credential::new(config(), KEY).unwrap();
        assert!(validate_pem(&format!("{}\nOCI_API_KEY", KEY.trim_end())).is_ok());
        let request_input = ApiRequestInput {
            method: "GET".into(),
            ..Default::default()
        };
        let url = Url::parse(&format!(
            "https://{HOST}{INSTANCES_PATH}?compartmentId=ocid1.compartment.oc1..test&page=a%2Bb+c"
        ))
        .unwrap();
        let (request, header) = prepare(&credential, url, &request_input, DATE).unwrap();
        let signature = header
            .to_str()
            .unwrap()
            .rsplit_once("signature=\"")
            .unwrap()
            .1
            .trim_end_matches('"');
        assert_eq!(
            signature,
            include_str!("../tests/fixtures/oci/list-instances.signature").trim()
        );
        assert_eq!(
            request.url().query().unwrap(),
            "compartmentId=ocid1.compartment.oc1..test&page=a%2Bb%20c"
        );
        assert_eq!(request.headers()["x-date"], DATE);
        assert!(header.is_sensitive());
    }

    #[test]
    fn oci_key_bom_import_preserves_the_signature() {
        let pem = format!(" \n\u{feff}{}\r\nOCI_API_KEY\r\n", KEY.replace('\n', "\r\n"));
        let credential = Credential::new(config(), &pem).unwrap();
        let input = ApiRequestInput {
            method: "GET".into(),
            ..Default::default()
        };
        let url = Url::parse(&format!(
            "https://{HOST}{INSTANCES_PATH}?compartmentId=ocid1.compartment.oc1..test&page=a%2Bb+c"
        ))
        .unwrap();
        let (_, header) = prepare(&credential, url, &input, DATE).unwrap();
        let signature = header
            .to_str()
            .unwrap()
            .rsplit_once("signature=\"")
            .unwrap()
            .1
            .trim_end_matches('"');
        assert_eq!(
            signature,
            include_str!("../tests/fixtures/oci/list-instances.signature").trim()
        );
    }

    #[test]
    fn oci_key_errors_classify_invalid_input_without_echoing_it() {
        for (pem, expected) in [
            (String::new(), "OCI_KEY_EMPTY"),
            ("OCI-KEY-ERROR-MARKER".into(), "OCI_KEY_FORMAT"),
            (
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nOCI-KEY-ERROR-MARKER\n-----END ENCRYPTED PRIVATE KEY-----".into(),
                "OCI_KEY_ENCRYPTED",
            ),
            (
                "-----BEGIN PRIVATE KEY-----\nOCI-KEY-ERROR-MARKER\n-----END PRIVATE KEY-----".into(),
                "OCI_KEY_BASE64",
            ),
            (
                format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----", STANDARD.encode(b"OCI-KEY-ERROR-MARKER")),
                "OCI_KEY_INVALID",
            ),
            (
                "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n-----END PRIVATE KEY-----".into(),
                "OCI_KEY_ALGORITHM",
            ),
            ("x".repeat(64 * 1024 + 1), "OCI_KEY_TOO_LARGE"),
        ] {
            let error = validate_pem(&pem).unwrap_err().to_string();
            assert!(error.starts_with(expected), "{error}");
            assert!(!error.contains("OCI-KEY-ERROR-MARKER"));
            assert!(!error.contains("MC4CAQAw"));
        }
    }

    #[test]
    fn oci_rejects_targets_options_and_keys_outside_the_first_release() {
        let config = config();
        let mut credential = Credential::new(config.clone(), KEY).unwrap();
        credential.forget_key();
        let get = ApiRequestInput {
            method: "GET".into(),
            ..Default::default()
        };
        let valid = format!("https://{HOST}{INSTANCES_PATH}?compartmentId=test");
        for (url, error) in [
            (valid.replace("https:", "http:"), "outside"),
            (
                valid.replace(HOST, "iaas.ap-osaka-1.oraclecloud.com"),
                "outside",
            ),
            (
                valid.replace(HOST, "iaas.ap-tokyo-1.oraclecloud.com.example.org"),
                "outside",
            ),
            (valid.replace(HOST, &format!("{HOST}:444")), "outside"),
            (valid.replace(HOST, &format!("user@{HOST}")), "outside"),
            (format!("{valid}#fragment"), "outside"),
            (valid.replace("instances", "vcns"), "only GET"),
            (valid.replace("instances", "instances/test"), "only GET"),
            (
                valid.replace("?compartmentId=test", ""),
                "requires compartmentId",
            ),
            (format!("{valid}&%63ompartmentId=other"), "Duplicate"),
            (format!("{valid}&page="), "Empty"),
            (format!("{valid}&limit=0"), "Invalid OCI limit"),
            (format!("{valid}&unknown=x"), "Unsupported"),
        ] {
            let actual = prepare(&credential, Url::parse(&url).unwrap(), &get, DATE)
                .unwrap_err()
                .to_string();
            assert!(actual.contains(error), "{actual}");
        }
        for header in ["HOST", "authorization", "DaTe", "X-DaTe"] {
            let mut input = get.clone();
            input.headers.insert(header.into(), "override".into());
            assert!(
                prepare(&credential, Url::parse(&valid).unwrap(), &input, DATE)
                    .unwrap_err()
                    .to_string()
                    .contains("override")
            );
        }
        for method in ["POST", "PUT", "DELETE", "HEAD"] {
            let input = ApiRequestInput {
                method: method.into(),
                ..Default::default()
            };
            assert!(
                prepare(&credential, Url::parse(&valid).unwrap(), &input, DATE)
                    .unwrap_err()
                    .to_string()
                    .contains("only GET")
            );
        }
        for input in [
            ApiRequestInput {
                body: Some(serde_json::json!("data")),
                ..get.clone()
            },
            ApiRequestInput {
                body_base64: Some("".into()),
                ..get.clone()
            },
            ApiRequestInput {
                form: [("x".into(), serde_json::json!(1))].into(),
                ..get.clone()
            },
            ApiRequestInput {
                files: vec![crate::executor::ApiUploadFile {
                    field: "file".into(),
                    path: "unused".into(),
                    file_name: None,
                    content_type: None,
                }],
                ..get.clone()
            },
        ] {
            assert!(
                prepare(&credential, Url::parse(&valid).unwrap(), &input, DATE)
                    .unwrap_err()
                    .to_string()
                    .contains("body")
            );
        }
        assert!(
            prepare(&credential, Url::parse(&valid).unwrap(), &get, DATE)
                .unwrap_err()
                .to_string()
                .contains("Reimport")
        );
        let mut unknown = config;
        unknown.region = "example.org/".into();
        assert!(unknown.base_url().is_err());
        for invalid in [
            "broken PEM",
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nZm9v\n-----END ENCRYPTED PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n-----END PRIVATE KEY-----",
        ] {
            assert!(validate_pem(invalid).is_err());
        }
    }

    #[test]
    fn oci_key_is_sign_only_and_not_portable() {
        let dir = tempfile::tempdir().unwrap();
        let vault = crate::vault::Vault::open(dir.path().join("source")).unwrap();
        let mut input: crate::model::ConnectionInput = serde_json::from_value(serde_json::json!({
            "name": "OCI test"
        }))
        .unwrap();
        let saved = vault
            .save_oci_connection(input.clone(), None, config(), Some(KEY), None)
            .unwrap();
        let item = vault.get_connection(saved.id).unwrap();
        assert_eq!(item.stored.capabilities(&item.secrets), ["http"]);
        assert!(item.secrets.non_empty_values().is_empty());
        assert!(item.secrets.get("privateKey").is_none());
        assert!(item.secrets.get("oci").is_none());
        assert!(
            vault
                .owner_secret_view(saved.id)
                .unwrap()
                .fields
                .iter()
                .all(|f| !f.value.contains("PRIVATE KEY"))
        );
        assert!(
            !std::fs::read_to_string(vault.data_dir().join("vault.json"))
                .unwrap()
                .contains("PRIVATE KEY")
        );
        input.id = Some(saved.id);
        assert!(
            vault
                .save_oci_connection(
                    input,
                    Some(&saved.updated_at),
                    config(),
                    Some("bad replacement"),
                    None,
                )
                .is_err()
        );
        assert!(
            vault
                .get_connection(saved.id)
                .unwrap()
                .secrets
                .oci
                .as_ref()
                .unwrap()
                .is_configured()
        );
        let cloud = crate::backup::cloud_snapshot(&vault).unwrap();
        assert!(!String::from_utf8_lossy(&cloud).contains("PRIVATE KEY"));
        let target = crate::vault::Vault::open(dir.path().join("cloud-restored")).unwrap();
        crate::backup::restore_cloud_snapshot(&target, &cloud).unwrap();
        let portable = dir.path().join("portable.mvault");
        crate::backup::export_to_file(&vault, &portable).unwrap();
        let portable_target =
            crate::vault::Vault::open(dir.path().join("portable-restored")).unwrap();
        crate::backup::import_from_file(&portable_target, &portable).unwrap();
        let protected = dir.path().join("protected.mvault");
        crate::backup::export_protected(&vault, &protected, "test backup password").unwrap();
        let protected_target =
            crate::vault::Vault::open(dir.path().join("protected-restored")).unwrap();
        crate::backup::import_with_password(&protected_target, &protected, "test backup password")
            .unwrap();
        for target in [target, portable_target, protected_target] {
            let restored = target.get_connection(saved.id).unwrap();
            let credential = restored.secrets.oci.as_ref().unwrap();
            assert_eq!(credential.config, config());
            assert!(!credential.is_configured());
            assert!(restored.stored.capabilities(&restored.secrets).is_empty());
        }
    }
}
