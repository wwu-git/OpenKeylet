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
use std::time::Duration;
use sha2::{Digest, Sha256};
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
        let mut signed_headers = "(request-target) host x-date".to_owned();
        let mut message = format!(
            "(request-target): {} {target}\nhost: {host}\nx-date: {date}",
            request.method().as_str().to_ascii_lowercase()
        );
        let content = matches!(
            *request.method(),
            Method::POST | Method::PUT | Method::PATCH
        ) || request.body().is_some();
        if content {
            let bytes = request
                .body()
                .map(|body| {
                    body.as_bytes()
                        .ok_or_else(|| anyhow!("OCI signing requires a buffered request body"))
                })
                .transpose()?
                .unwrap_or_default();
            let length = bytes.len();
            let digest = STANDARD.encode(Sha256::digest(bytes));
            if !request.headers().contains_key("content-type") {
                request.headers_mut().insert(
                    "content-type",
                    HeaderValue::from_static("application/octet-stream"),
                );
            }
            let content_type = request.headers()["content-type"]
                .to_str()
                .map_err(|_| anyhow!("Invalid OCI Content-Type"))?;
            message.push_str(&format!(
                "\nx-content-sha256: {digest}\ncontent-type: {content_type}\ncontent-length: {length}"
            ));
            signed_headers.push_str(" x-content-sha256 content-type content-length");
            request
                .headers_mut()
                .insert("x-content-sha256", HeaderValue::from_str(&digest)?);
            request.headers_mut().insert(
                "content-length",
                HeaderValue::from_str(&length.to_string())?,
            );
        }
        let mut signature = vec![0; key.public().modulus_len()];
        key.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            message.as_bytes(),
            &mut signature,
        )
        .map_err(|_| anyhow!("OCI request signing failed"))?;
        let mut header = HeaderValue::from_str(&format!(
            "Signature version=\"1\",keyId=\"{}/{}/{}\",algorithm=\"rsa-sha256\",headers=\"{signed_headers}\",signature=\"{}\"",
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
    ensure!(
        pem.len() <= 64 * 1024,
        "OCI_KEY_TOO_LARGE: The signing key exceeds the 64 KiB limit"
    );
    let pem = pem.trim();
    let pem = pem.strip_prefix('\u{feff}').unwrap_or(pem).trim();
    ensure!(
        !pem.is_empty(),
        "OCI_KEY_EMPTY: Enter the complete RSA private key"
    );
    ensure!(
        !pem.starts_with("-----BEGIN ENCRYPTED PRIVATE KEY-----")
            && !pem
                .lines()
                .any(|line| line.trim().eq_ignore_ascii_case("Proc-Type: 4,ENCRYPTED")),
        "OCI_KEY_ENCRYPTED: Encrypted private keys are not supported; import an unencrypted RSA PEM"
    );
    // Oracle's downloadable API keys may include this marker after the PEM.
    let pem = pem.strip_suffix("\nOCI_API_KEY").map_or(pem, str::trim_end);
    for (label, pkcs8) in [("PRIVATE KEY", true), ("RSA PRIVATE KEY", false)] {
        if let Some(data) = pem
            .strip_prefix(&format!("-----BEGIN {label}-----"))
            .and_then(|body| body.strip_suffix(&format!("-----END {label}-----")))
        {
            let data = Zeroizing::new(data.split_ascii_whitespace().collect::<String>());
            let der = Zeroizing::new(STANDARD.decode(data.as_bytes()).map_err(|_| {
                anyhow!("OCI_KEY_BASE64: The PEM body is not valid Base64; copy the complete original key")
            })?);
            return (if pkcs8 {
                RsaKeyPair::from_pkcs8(&der)
            } else {
                RsaKeyPair::from_der(&der)
            })
            .map_err(|rejected| {
                anyhow!(match rejected.to_string().as_str() {
                    "WrongAlgorithm" => "OCI_KEY_ALGORITHM: OCI signing requires an RSA key with a supported algorithm",
                    "TooSmall" => "OCI_KEY_TOO_SMALL: RSA key parameters are below the supported minimum; use a key of at least 2048 bits",
                    "TooLarge" | "VersionNotSupported" | "PrivateModulusLenNotMultipleOf512Bits" => "OCI_KEY_UNSUPPORTED: The RSA key parameters or version are not supported",
                    _ => "OCI_KEY_INVALID: The private key could not pass RSA validation; download the original API signing key again",
                })
            });
        }
    }
    bail!("OCI_KEY_FORMAT: Import a complete RSA PEM private key; OpenSSH format is not supported")
}

pub(crate) fn validate_request(config: &Config, request: &Request) -> Result<()> {
    config.validate()?;
    let url = request.url();
    let host = url.host_str().unwrap_or_default();
    let prefix = [
        format!(".{}.oraclecloud.com", config.region),
        format!(".{}.oci.oraclecloud.com", config.region),
    ]
    .into_iter()
    .find_map(|suffix| host.strip_suffix(&suffix));
    let valid_prefix = prefix.is_some_and(|prefix| {
        !prefix.is_empty()
            && prefix.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && label.as_bytes()[0].is_ascii_alphanumeric()
                    && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                    && label
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
    });
    ensure!(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && valid_prefix,
        "OCI request target is outside the configured region"
    );
    ensure!(
        !matches!(*request.method(), Method::CONNECT | Method::TRACE),
        "OCI transport does not support CONNECT or TRACE"
    );
    ensure!(
        request.headers().get_all("content-type").iter().count() <= 1,
        "OCI request cannot have duplicate Content-Type headers"
    );
    ensure!(
        !request.headers().keys().any(|name| matches!(
            name.as_str(),
            "host"
                | "authorization"
                | "date"
                | "x-date"
                | "x-content-sha256"
                | "content-length"
                | "transfer-encoding"
                | "proxy-authorization"
        )),
        "OCI request cannot override signing or transport headers"
    );
    Ok(())
}

pub(crate) fn client_builder(timeout: Option<u64>) -> reqwest::ClientBuilder {
    Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_secs(
            timeout.filter(|t| *t > 0).unwrap_or(30),
        ))
}

pub(crate) fn prepare(
    credential: &Credential,
    mut request: Request,
    date: &str,
) -> Result<(Request, HeaderValue)> {
    validate_request(&credential.config, &request)?;
    let signature = credential.sign(&mut request, date)?;
    Ok((request, signature))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
        let url = Url::parse(&format!(
            "https://{HOST}{INSTANCES_PATH}?compartmentId=ocid1.compartment.oc1..test&page=a%2Bb%20c"
        ))
        .unwrap();
        let (request, header) = prepare(&credential, Request::new(Method::GET, url), DATE).unwrap();
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
        let pem = format!(
            " \n\u{feff}{}\r\nOCI_API_KEY\r\n",
            KEY.replace('\n', "\r\n")
        );
        let credential = Credential::new(config(), &pem).unwrap();
        let url = Url::parse(&format!(
            "https://{HOST}{INSTANCES_PATH}?compartmentId=ocid1.compartment.oc1..test&page=a%2Bb%20c"
        ))
        .unwrap();
        let (_, header) = prepare(&credential, Request::new(Method::GET, url), DATE).unwrap();
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
    fn oci_request_target_and_transport_headers_are_validated() {
        let credential = Credential::new(config(), KEY).unwrap();
        for host in [
            HOST,
            "identity.ap-tokyo-1.oci.oraclecloud.com",
            "new-service.ap-tokyo-1.oraclecloud.com",
            "namespace.compat.objectstorage.ap-tokyo-1.oraclecloud.com",
        ] {
            let request = Request::new(
                Method::GET,
                Url::parse(&format!(
                    "https://{host}/future/api?newParam=1&newParam=2&literal=+&encoded=%2F"
                ))
                .unwrap(),
            );
            let (signed, _) = prepare(&credential, request, DATE).unwrap();
            assert_eq!(
                signed.url().query().unwrap(),
                "newParam=1&newParam=2&literal=+&encoded=%2F"
            );
        }
        for target in [
            "http://iaas.ap-tokyo-1.oraclecloud.com/future",
            "https://iaas.ap-osaka-1.oraclecloud.com/future",
            "https://iaas.ap-tokyo-1.oraclecloud.com.example.org/future",
            "https://iaas.ap-tokyo-1.oraclecloud.com:444/future",
            "https://user@iaas.ap-tokyo-1.oraclecloud.com/future",
            "https://iaas.ap-tokyo-1.oraclecloud.com/future#fragment",
            "https://127.0.0.1/future",
            "https://ap-tokyo-1.oraclecloud.com/future",
            "https://bad_label.ap-tokyo-1.oraclecloud.com/future",
            "https://iaas.ap-tokyo-1.oraclecloud.com./future",
        ] {
            assert!(
                prepare(
                    &credential,
                    Request::new(Method::GET, Url::parse(target).unwrap()),
                    DATE
                )
                .unwrap_err()
                .to_string()
                .contains("outside")
            );
        }
        let url = Url::parse(&format!("https://{HOST}/arbitrary/path")).unwrap();
        for header in [
            "HOST",
            "authorization",
            "DaTe",
            "X-DaTe",
            "x-content-sha256",
            "content-length",
            "transfer-encoding",
            "proxy-authorization",
        ] {
            let mut request = Request::new(Method::GET, url.clone());
            request.headers_mut().insert(
                reqwest::header::HeaderName::from_bytes(header.as_bytes()).unwrap(),
                HeaderValue::from_static("override"),
            );
            assert!(
                prepare(&credential, request, DATE)
                    .unwrap_err()
                    .to_string()
                    .contains("override")
            );
        }
        for method in [Method::CONNECT, Method::TRACE] {
            assert!(prepare(&credential, Request::new(method, url.clone()), DATE).is_err());
        }
        for method in [
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::HEAD,
            Method::OPTIONS,
            Method::from_bytes(b"NEW-METHOD").unwrap(),
        ] {
            assert!(prepare(&credential, Request::new(method, url.clone()), DATE).is_ok());
        }
        let mut credential = credential;
        credential.forget_key();
        assert!(
            prepare(&credential, Request::new(Method::GET, url), DATE)
                .unwrap_err()
                .to_string()
                .contains("Reimport")
        );
        let mut unknown = config();
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
