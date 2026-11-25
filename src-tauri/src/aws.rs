use crate::{
    api_provider::{AwsEndpoint, Endpoint},
    model::{AwsModuleBindings, ItemModule, SecretBundle, StoredConnection},
};
use anyhow::{Context, Result, bail, ensure};
use hmac::{Hmac, Mac};
use reqwest::{
    Request,
    header::{HeaderMap, HeaderName, HeaderValue},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

pub(crate) const AUTH_MODE: &str = "aws-sigv4";
pub(crate) const MAX_SECRET_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 64 * 1024 * 1024;
const RESERVED_HEADERS: &[&str] = &[
    "authorization",
    "host",
    "x-amz-date",
    "x-amz-security-token",
    "x-amz-content-sha256",
    "content-length",
    "transfer-encoding",
    "proxy-authorization",
    "proxy-connection",
    "connection",
];

pub(crate) struct Config {
    pub region: String,
}

pub(crate) struct Credential {
    pub config: Config,
    access_key_id: Zeroizing<String>,
    secret_access_key: Zeroizing<String>,
    session_token: Option<Zeroizing<String>>,
}

fn module(modules: &[ItemModule], id: Option<Uuid>) -> Option<&ItemModule> {
    modules.iter().find(|module| Some(module.id) == id)
}

fn bound_secret<'a>(
    modules: &'a [ItemModule],
    secrets: &'a SecretBundle,
    id: Option<Uuid>,
) -> Option<&'a str> {
    module(modules, id)?
        .secret_name()
        .and_then(|name| secrets.get(name))
}

pub(crate) fn region_endpoint(region: &str) -> Result<AwsEndpoint> {
    ensure!(
        region.len() <= 64
            && region
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "Invalid AWS region"
    );
    match crate::api_provider::identify(&Url::parse(&format!(
        "https://ec2.{region}.amazonaws.com/"
    ))?)? {
        Endpoint::Aws(endpoint) => Ok(endpoint),
        _ => bail!("Unsupported AWS commercial region"),
    }
}

pub(crate) fn configured_region(item: &StoredConnection, secrets: &SecretBundle) -> Result<String> {
    let bindings = item
        .aws_bindings
        .as_ref()
        .context("AWS authentication bindings are missing")?;
    let region = bound_secret(&item.modules, secrets, bindings.region)
        .filter(|value| !value.trim().is_empty())
        .context("AWS configuration is incomplete: region")?
        .trim();
    Ok(region_endpoint(region)?.region)
}

pub(crate) fn validate_modules(
    modules: &[ItemModule],
    secrets: &SecretBundle,
    bindings: Option<&AwsModuleBindings>,
) -> Result<()> {
    let Some(bindings) = bindings else {
        return Ok(());
    };
    let ids: Vec<_> = [
        bindings.service_url,
        bindings.access_key_id,
        bindings.secret_access_key,
        bindings.session_token,
        bindings.region,
    ]
    .into_iter()
    .flatten()
    .collect();
    ensure!(
        ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len(),
        "Each AWS authentication field must bind a different module"
    );
    if let Some(module) = module(modules, bindings.service_url) {
        ensure!(
            module.kind == "url",
            "AWS service URL must bind a URL module"
        );
        if !module.value.trim().is_empty() {
            ensure!(
                matches!(
                    crate::api_provider::identify(
                        &Url::parse(module.value.trim())
                            .map_err(|_| anyhow::anyhow!("Invalid AWS service URL"))?
                    )?,
                    Endpoint::Aws(_)
                ),
                "Unsupported AWS service endpoint"
            );
        }
    }
    for id in [
        bindings.access_key_id,
        bindings.secret_access_key,
        bindings.session_token,
        bindings.region,
    ] {
        if let Some(module) = module(modules, id) {
            ensure!(
                module.is_secret(),
                "AWS credential and region roles must bind secret modules"
            );
            if let Some(value) = bound_secret(modules, secrets, id) {
                ensure!(
                    value.len() <= MAX_SECRET_BYTES,
                    "AWS authentication field is too large"
                );
            }
        }
    }
    if let Some(region) =
        bound_secret(modules, secrets, bindings.region).filter(|value| !value.trim().is_empty())
    {
        region_endpoint(region.trim())?;
    }
    Ok(())
}

impl Credential {
    pub(crate) fn from_modules(item: &StoredConnection, secrets: &SecretBundle) -> Result<Self> {
        let bindings = item
            .aws_bindings
            .as_ref()
            .context("AWS authentication bindings are missing")?;
        validate_modules(&item.modules, secrets, Some(bindings))?;
        let read = |id, role| -> Result<&str> {
            bound_secret(&item.modules, secrets, id)
                .filter(|value| !value.is_empty())
                .with_context(|| format!("AWS configuration is incomplete: {role}"))
        };
        let url_module = module(&item.modules, bindings.service_url)
            .filter(|module| module.kind == "url" && !module.value.trim().is_empty())
            .context("AWS configuration is incomplete: service URL")?;
        let endpoint = match crate::api_provider::identify(
            &Url::parse(&url_module.value)
                .map_err(|_| anyhow::anyhow!("Invalid AWS service URL"))?,
        )? {
            Endpoint::Aws(endpoint) => endpoint,
            _ => bail!("Unsupported AWS service endpoint"),
        };
        let region = configured_region(item, secrets)?;
        ensure!(
            endpoint.region == region,
            "AWS service URL and configured region do not match"
        );
        let access_key_id = read(bindings.access_key_id, "Access Key ID")?;
        ensure!(
            access_key_id.len() <= 128
                && access_key_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric()),
            "Invalid AWS Access Key ID"
        );
        let secret_access_key = read(bindings.secret_access_key, "Secret Access Key")?;
        ensure!(
            secret_access_key.len() <= MAX_SECRET_BYTES
                && secret_access_key.is_ascii()
                && !secret_access_key.chars().any(char::is_control)
                && secret_access_key.trim() == secret_access_key,
            "Invalid AWS Secret Access Key"
        );
        let session_token = if bindings.session_token.is_some() {
            let module = module(&item.modules, bindings.session_token)
                .context("AWS session token binding is unavailable")?;
            ensure!(
                module.is_secret(),
                "AWS session token must bind a secret module"
            );
            bound_secret(&item.modules, secrets, bindings.session_token)
                .filter(|value| !value.is_empty())
        } else {
            None
        };
        if let Some(value) = session_token {
            ensure!(
                value.len() <= MAX_SECRET_BYTES
                    && value.is_ascii()
                    && !value.chars().any(char::is_control),
                "Invalid AWS session token"
            );
        }
        ensure!(
            !access_key_id.starts_with("ASIA") || session_token.is_some(),
            "AWS temporary credentials require a Session Token"
        );
        Ok(Self {
            config: Config { region },
            access_key_id: Zeroizing::new(access_key_id.to_owned()),
            secret_access_key: Zeroizing::new(secret_access_key.to_owned()),
            session_token: session_token.map(|value| Zeroizing::new(value.to_owned())),
        })
    }
}

pub(crate) fn validate_request(request: &Request) -> Result<()> {
    ensure!(
        matches!(
            crate::api_provider::identify(request.url())?,
            Endpoint::Aws(_)
        ),
        "Unsupported AWS request endpoint"
    );
    ensure!(
        request.url().as_str().len() <= 1024 * 1024,
        "AWS request URL is too large"
    );
    for name in RESERVED_HEADERS {
        ensure!(
            !request.headers().contains_key(*name),
            "AWS authentication or transport headers are generated locally"
        );
    }
    ensure!(
        !request
            .headers()
            .get(reqwest::header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.to_ascii_lowercase().contains("aws-chunked")),
        "AWS streaming signatures are not supported"
    );
    validate_query_encoding(request.url())?;
    for (name, _) in request.url().query_pairs() {
        ensure!(
            !name.to_ascii_lowercase().starts_with("x-amz-")
                && !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "awsaccesskeyid" | "signature" | "signatureversion" | "securitytoken"
                ),
            "AWS presigned or caller-authenticated query parameters are not supported"
        );
    }
    ensure!(
        !request.method().as_str().chars().any(char::is_control),
        "Invalid AWS request method"
    );
    Ok(())
}

fn encode(bytes: &[u8], slash: bool) -> String {
    let mut output = String::with_capacity(bytes.len());
    for byte in bytes {
        if byte.is_ascii_alphanumeric()
            || matches!(*byte, b'-' | b'_' | b'.' | b'~')
            || slash && *byte == b'/'
        {
            output.push(*byte as char);
        } else {
            use std::fmt::Write;
            write!(&mut output, "%{byte:02X}").expect("writing a String");
        }
    }
    output
}

fn canonical_path(path: &str) -> String {
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            value => segments.push(value),
        }
    }
    let mut result = format!("/{}", segments.join("/"));
    if path.ends_with('/') && result != "/" {
        result.push('/');
    }
    encode(result.as_bytes(), true)
}

fn validate_query_encoding(url: &Url) -> Result<()> {
    for component in url
        .query()
        .unwrap_or_default()
        .split('&')
        .flat_map(|pair| pair.splitn(2, '='))
    {
        let bytes = component.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                ensure!(index + 2 < bytes.len(), "Invalid AWS query encoding");
                let digit = |value: u8| match value {
                    b'0'..=b'9' => Some(value - b'0'),
                    b'a'..=b'f' => Some(value - b'a' + 10),
                    b'A'..=b'F' => Some(value - b'A' + 10),
                    _ => None,
                };
                let high = digit(bytes[index + 1]).context("Invalid AWS query encoding")?;
                let low = digit(bytes[index + 2]).context("Invalid AWS query encoding")?;
                decoded.push(high * 16 + low);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        std::str::from_utf8(&decoded).context("AWS query parameters must use valid UTF-8")?;
    }
    Ok(())
}

fn normalize_query(url: &mut Url) -> String {
    let mut pairs: Vec<_> = url
        .query_pairs()
        .map(|(name, value)| {
            (
                encode(name.as_bytes(), false),
                encode(value.as_bytes(), false),
            )
        })
        .collect();
    pairs.sort();
    let query = pairs
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    query
}

fn canonical_headers(headers: &HeaderMap) -> Result<(String, String)> {
    let mut canonical = BTreeMap::new();
    for name in headers.keys() {
        if matches!(
            name.as_str(),
            "authorization" | "expect" | "transfer-encoding" | "user-agent" | "x-amzn-trace-id"
        ) {
            continue;
        }
        let values = headers
            .get_all(name)
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .map(|text| text.split_ascii_whitespace().collect::<Vec<_>>().join(" "))
            })
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("Invalid AWS request header")?;
        canonical.insert(name.as_str(), values.join(","));
    }
    let names = canonical.keys().copied().collect::<Vec<_>>().join(";");
    let values = canonical
        .into_iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect::<String>();
    Ok((values, names))
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hmac(key: &[u8], value: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts keys of every length");
    mac.update(value);
    Zeroizing::new(mac.finalize().into_bytes().to_vec())
}
fn signature(secret: &str, date: &str, region: &str, service: &str, canonical: &str) -> String {
    let initial = Zeroizing::new(format!("AWS4{secret}"));
    let day = hmac(initial.as_bytes(), date[..8].as_bytes());
    let region_key = hmac(&day, region.as_bytes());
    let service_key = hmac(&region_key, service.as_bytes());
    let signing_key = hmac(&service_key, b"aws4_request");
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{}/{region}/{service}/aws4_request\n{}",
        &date[..8],
        hash(canonical.as_bytes())
    );
    hmac(&signing_key, to_sign.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sensitive(value: &str) -> Result<HeaderValue> {
    let mut value = HeaderValue::from_str(value)
        .map_err(|_| anyhow::anyhow!("Invalid AWS authentication header"))?;
    value.set_sensitive(true);
    Ok(value)
}

pub(crate) fn prepare(
    credential: &Credential,
    endpoint: &AwsEndpoint,
    mut request: Request,
    date: &str,
) -> Result<(Request, HeaderValue)> {
    validate_request(&request)?;
    ensure!(
        date.len() == 16 && chrono::NaiveDateTime::parse_from_str(date, "%Y%m%dT%H%M%SZ").is_ok(),
        "Invalid AWS signing timestamp"
    );
    let actual = match crate::api_provider::identify(request.url())? {
        Endpoint::Aws(value) => value,
        _ => bail!("Unsupported AWS request endpoint"),
    };
    ensure!(
        actual.region == endpoint.region
            && actual.signing_service == endpoint.signing_service
            && endpoint.region == credential.config.region,
        "AWS signing service or region does not match the configured endpoint"
    );
    let body = request
        .body()
        .map(|body| {
            body.as_bytes()
                .context("AWS request body must be buffered before signing")
        })
        .transpose()?
        .unwrap_or_default();
    ensure!(
        body.len() <= MAX_BODY_BYTES,
        "AWS request body exceeds 64 MiB"
    );
    let payload_hash = hash(body);
    let query = normalize_query(request.url_mut());
    let host = request
        .url()
        .host_str()
        .context("Invalid AWS request host")?
        .to_owned();
    request
        .headers_mut()
        .insert(reqwest::header::HOST, HeaderValue::from_str(&host)?);
    request.headers_mut().insert(
        HeaderName::from_static("x-amz-date"),
        HeaderValue::from_str(date)?,
    );
    request.headers_mut().insert(
        HeaderName::from_static("x-amz-content-sha256"),
        HeaderValue::from_str(&payload_hash)?,
    );
    if let Some(token) = &credential.session_token {
        request.headers_mut().insert(
            HeaderName::from_static("x-amz-security-token"),
            sensitive(token)?,
        );
    }
    let (headers, signed_headers) = canonical_headers(request.headers())?;
    let headers = Zeroizing::new(headers);
    let canonical = Zeroizing::new(format!(
        "{}\n{}\n{query}\n{}\n{signed_headers}\n{payload_hash}",
        request.method().as_str(),
        canonical_path(request.url().path()),
        headers.as_str()
    ));
    let result = Zeroizing::new(signature(
        &credential.secret_access_key,
        date,
        &endpoint.region,
        endpoint.signing_service,
        &canonical,
    ));
    let authorization = Zeroizing::new(format!(
        "AWS4-HMAC-SHA256 Credential={}/{}/{}/{}/aws4_request, SignedHeaders={signed_headers}, Signature={}",
        credential.access_key_id.as_str(),
        &date[..8],
        endpoint.region,
        endpoint.signing_service,
        result.as_str()
    ));
    let authorization = sensitive(&authorization)?;
    request
        .headers_mut()
        .insert(reqwest::header::AUTHORIZATION, authorization.clone());
    Ok((request, authorization))
}

#[cfg(test)]
#[path = "aws_tests.rs"]
mod tests;
