use crate::model::{ItemModule, SecretBundle, StoredConnection};
use anyhow::{Result, bail, ensure};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue};
use schemars::JsonSchema;
use serde::Serialize;
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ProviderId {
    Aws,
    Oci,
    Github,
    Cloudflare,
    DigitalOcean,
    Hetzner,
    Azure,
    Gcp,
    Alibaba,
    Tencent,
    Huawei,
    Ovh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AwsEndpoint {
    pub region: String,
    pub signing_service: &'static str,
    pub service: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Generic,
    Aws(AwsEndpoint),
    Oci {
        region: String,
    },
    Token(ProviderId),
    Unsupported {
        provider: ProviderId,
        service: &'static str,
        authentication: &'static str,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionPlan {
    Generic,
    Oci,
    Aws(AwsEndpoint),
    Token(ProviderId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConfigurationStatus {
    Ready,
    NeedsConfiguration,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ServiceCapability {
    pub provider: ProviderId,
    pub service: String,
    pub authentication: String,
    pub status: ConfigurationStatus,
    pub missing_fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    pub documentation_urls: Vec<String>,
    pub instructions: String,
    pub limitations: Vec<String>,
}

pub(crate) const AWS_REGIONS: &[&str] = &[
    "us-east-1",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "af-south-1",
    "ap-east-1",
    "ap-east-2",
    "ap-south-1",
    "ap-south-2",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-northeast-3",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-southeast-3",
    "ap-southeast-4",
    "ap-southeast-5",
    "ap-southeast-6",
    "ap-southeast-7",
    "ca-central-1",
    "ca-west-1",
    "eu-central-1",
    "eu-central-2",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "il-central-1",
    "mx-central-1",
    "me-south-1",
    "me-central-1",
    "sa-east-1",
];

const AWS_SERVICES: &[(&str, &str)] = &[
    ("ec2", "EC2"),
    ("sts", "STS"),
    ("ssm", "Systems Manager"),
    ("secretsmanager", "Secrets Manager"),
    ("logs", "CloudWatch Logs"),
    ("monitoring", "CloudWatch"),
    ("elasticloadbalancing", "Elastic Load Balancing"),
    ("kms", "KMS"),
    ("dynamodb", "DynamoDB"),
];

fn domain(host: &str, suffix: &str) -> bool {
    host == suffix
        || host
            .strip_suffix(suffix)
            .is_some_and(|prefix| prefix.ends_with('.'))
}

fn secure_url(url: &Url) -> Result<()> {
    ensure!(
        !url.host_str().is_some_and(|host| host.ends_with('.')),
        "Native API URLs require a canonical hostname without a trailing dot"
    );
    ensure!(
        url.scheme() == "https" && url.port_or_known_default() == Some(443),
        "Native API services require HTTPS on port 443"
    );
    ensure!(
        url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
        "Native API URLs cannot contain user information or a fragment"
    );
    Ok(())
}

fn path_prefix(url: &Url, prefix: &str) -> Result<()> {
    ensure!(
        url.path() == prefix || url.path().starts_with(&format!("{prefix}/")),
        "URL is outside the supported provider API path"
    );
    Ok(())
}

pub fn identify(url: &Url) -> Result<Endpoint> {
    let Some(host) = url.host_str() else {
        bail!("API URL has no host")
    };
    let host = host.trim_end_matches('.');
    let fixed = match host {
        "api.github.com" => Some((ProviderId::Github, "")),
        "api.cloudflare.com" => Some((ProviderId::Cloudflare, "/client/v4")),
        "api.digitalocean.com" => Some((ProviderId::DigitalOcean, "/v2")),
        "api.hetzner.cloud" => Some((ProviderId::Hetzner, "/v1")),
        _ => None,
    };
    if let Some((provider, prefix)) = fixed {
        secure_url(url)?;
        if !prefix.is_empty() {
            path_prefix(url, prefix)?;
        }
        return Ok(Endpoint::Token(provider));
    }
    if domain(host, "amazonaws.com") || domain(host, "amazonaws.com.cn") || domain(host, "api.aws")
    {
        secure_url(url)?;
        if host == "iam.amazonaws.com" {
            return Ok(Endpoint::Aws(AwsEndpoint {
                region: "us-east-1".into(),
                signing_service: "iam",
                service: "IAM",
            }));
        }
        let labels = host
            .strip_suffix(".amazonaws.com")
            .unwrap_or_default()
            .split('.')
            .collect::<Vec<_>>();
        if let [service, region] = labels.as_slice() {
            if AWS_REGIONS.contains(region) {
                if let Some((signing_service, display)) =
                    AWS_SERVICES.iter().find(|(name, _)| name == service)
                {
                    return Ok(Endpoint::Aws(AwsEndpoint {
                        region: (*region).into(),
                        signing_service,
                        service: display,
                    }));
                }
            }
        }
        return Ok(Endpoint::Unsupported {
            provider: ProviderId::Aws,
            service: "Unsupported AWS endpoint",
            authentication: "unsupported-aws-signing",
            reason: "AWS endpoint is outside the supported standard commercial service/region table; S3, global STS, private/FIPS/dualstack endpoints, other partitions and SigV4a are not supported",
        });
    }
    if domain(host, "oraclecloud.com") {
        secure_url(url)?;
        let stem = host
            .strip_suffix(".oci.oraclecloud.com")
            .or_else(|| host.strip_suffix(".oraclecloud.com"))
            .unwrap_or_default();
        if let Some((service, region)) = stem.rsplit_once('.') {
            if !service.is_empty() && crate::oci::REGIONS.contains(&region) {
                return Ok(Endpoint::Oci {
                    region: region.into(),
                });
            }
        }
        return Ok(Endpoint::Unsupported {
            provider: ProviderId::Oci,
            service: "Unsupported OCI endpoint",
            authentication: "unsupported-oci-realm",
            reason: "Only supported OCI commercial OC1 regional endpoints are available",
        });
    }
    let unsupported = if domain(host, "digitaloceanspaces.com") {
        Some((
            ProviderId::DigitalOcean,
            "DigitalOcean Spaces is an S3 service, not the Bearer-authenticated control API",
        ))
    } else if matches!(
        host,
        "robot-ws.your-server.de" | "api.hetzner.com" | "dns.hetzner.com"
    ) {
        Some((
            ProviderId::Hetzner,
            "Hetzner Robot, Storage Boxes and legacy DNS use separate APIs and are not covered by the Cloud adapter",
        ))
    } else if host == "management.azure.com" {
        Some((
            ProviderId::Azure,
            "Azure ARM requires a correctly scoped Entra access token; native token acquisition is not implemented",
        ))
    } else if matches!(
        host,
        "compute.googleapis.com" | "cloudresourcemanager.googleapis.com"
    ) {
        Some((
            ProviderId::Gcp,
            "Google Cloud requires an appropriate OAuth access token; service-account JWT exchange and credential refresh are not implemented",
        ))
    } else if host == "ecs.aliyuncs.com"
        || host
            .strip_prefix("ecs.")
            .is_some_and(|h| domain(h, "aliyuncs.com"))
    {
        Some((
            ProviderId::Alibaba,
            "Alibaba ECS requires its ACS3 signing protocol, which is not implemented",
        ))
    } else if domain(host, "tencentcloudapi.com") {
        Some((
            ProviderId::Tencent,
            "Tencent Cloud API 3.0 requires its TC3 signing protocol, which is not implemented",
        ))
    } else if domain(host, "myhuaweicloud.com") {
        Some((
            ProviderId::Huawei,
            "Huawei Cloud requires its AK/SK signing protocol or IAM X-Auth-Token; native authentication is not implemented",
        ))
    } else if matches!(
        host,
        "api.ovh.com"
            | "ca.api.ovh.com"
            | "us.api.ovh.com"
            | "api.eu.ovhcloud.com"
            | "api.us.ovhcloud.com"
            | "api.ca.ovhcloud.com"
    ) {
        Some((
            ProviderId::Ovh,
            "OVHcloud requires Application Key, Application Secret, Consumer Key and its request signature; native signing is not implemented",
        ))
    } else {
        None
    };
    if let Some((provider, reason)) = unsupported {
        secure_url(url)?;
        let (service, authentication) = match provider {
            ProviderId::DigitalOcean => ("DigitalOcean Spaces", "external-s3-signing"),
            ProviderId::Hetzner => ("Hetzner non-Cloud API", "external-hetzner-service-auth"),
            _ => {
                let (service, authentication, _, _) = metadata(provider);
                (service, authentication)
            }
        };
        return Ok(Endpoint::Unsupported {
            provider,
            service,
            authentication,
            reason,
        });
    }
    Ok(Endpoint::Generic)
}

fn value<'a>(
    item: &'a StoredConnection,
    secrets: &'a SecretBundle,
    id: Option<Uuid>,
) -> Option<&'a str> {
    crate::oci::binding_value(&item.modules, secrets, id)
}

fn visible_module(item: &StoredConnection, id: Option<Uuid>) -> bool {
    item.modules
        .iter()
        .find(|module| Some(module.id) == id)
        .is_some_and(ItemModule::agent_visible)
}

fn saved_endpoint(item: &StoredConnection, secrets: &SecretBundle) -> Result<Endpoint> {
    let raw = if item.http_auth_type == crate::aws::AUTH_MODE {
        item.aws_bindings
            .as_ref()
            .and_then(|binding| value(item, secrets, binding.service_url))
            .unwrap_or(&item.base_url)
    } else {
        &item.base_url
    };
    if raw.is_empty() {
        return Ok(Endpoint::Generic);
    }
    let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("Invalid saved API service URL"))?;
    identify(&url)
}

fn external_oauth(provider: ProviderId) -> bool {
    matches!(provider, ProviderId::Azure | ProviderId::Gcp)
}

fn external_access_token(secrets: &SecretBundle) -> Result<()> {
    let token = secrets
        .get("apiCredential")
        .ok_or_else(|| anyhow::anyhow!("Configure an externally obtained OAuth access token"))?;
    ensure!(
        token.len() <= 64 * 1024
            && !token.starts_with("-----BEGIN")
            && token
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte)),
        "Expected an OAuth access token, not client credentials, JSON or a private key"
    );
    Ok(())
}

pub fn execution_plan(
    item: &StoredConnection,
    secrets: &SecretBundle,
    target: &Url,
) -> Result<ExecutionPlan> {
    let endpoint = identify(target)?;
    let saved = saved_endpoint(item, secrets)?;
    if let Endpoint::Unsupported {
        provider, reason, ..
    } = &saved
    {
        if external_oauth(*provider) && item.http_auth_type == "bearer" {
            ensure!(
                matches!(&endpoint, Endpoint::Unsupported { provider: requested, .. } if requested == provider),
                "Externally managed OAuth credentials cannot be sent to another provider or host"
            );
            external_access_token(secrets)?;
            return Ok(ExecutionPlan::Token(*provider));
        }
        bail!("{reason}");
    }
    if let Endpoint::Unsupported { reason, .. } = &endpoint {
        bail!("{reason}");
    }
    if item.http_auth_type == crate::aws::AUTH_MODE {
        let Endpoint::Aws(endpoint) = endpoint else {
            bail!("AWS credentials can only be sent to supported AWS endpoints")
        };
        let Endpoint::Aws(saved) = saved else {
            bail!("Configure a supported AWS default service URL")
        };
        let region = crate::aws::configured_region(item, secrets)?;
        ensure!(
            saved.region == region && endpoint.region == region,
            "AWS target region does not match the saved region"
        );
        return Ok(ExecutionPlan::Aws(endpoint));
    }
    if item.http_auth_type == crate::oci::AUTH_MODE {
        let Endpoint::Oci { .. } = endpoint else {
            bail!("OCI credentials can only be sent to supported OCI regional endpoints")
        };
        let credential = crate::oci::Credential::from_modules(item, secrets)?;
        crate::oci::validate_request(
            &credential.config,
            &reqwest::Request::new(reqwest::Method::GET, target.clone()),
        )?;
        return Ok(ExecutionPlan::Oci);
    }
    if matches!(endpoint, Endpoint::Aws(_)) || matches!(saved, Endpoint::Aws(_)) {
        bail!(
            "AWS endpoints require aws-sigv4 authentication and bound AWS modules; Bearer fallback is disabled"
        );
    }
    if matches!(endpoint, Endpoint::Oci { .. }) || matches!(saved, Endpoint::Oci { .. }) {
        bail!(
            "OCI endpoints require oci-signature authentication and bound OCI modules; Bearer fallback is disabled"
        );
    }
    if let Endpoint::Token(saved) = saved {
        ensure!(
            matches!(endpoint, Endpoint::Token(provider) if provider == saved),
            "Native provider credentials cannot be sent to a different provider or host"
        );
    }
    match endpoint {
        Endpoint::Token(provider) => {
            ensure!(
                matches!(item.http_auth_type.as_str(), "bearer" | "apiKey" | "custom"),
                "Choose an explicit supported token/header authentication mode for this provider"
            );
            Ok(ExecutionPlan::Token(provider))
        }
        Endpoint::Generic => Ok(ExecutionPlan::Generic),
        _ => unreachable!("native signing endpoints handled above"),
    }
}

pub fn apply_defaults(provider: ProviderId, headers: &mut HeaderMap) -> Result<()> {
    if provider == ProviderId::Github {
        headers
            .entry(ACCEPT)
            .or_insert(HeaderValue::from_static("application/vnd.github+json"));
        headers
            .entry("x-github-api-version")
            .or_insert(HeaderValue::from_static("2022-11-28"));
    }
    Ok(())
}

fn metadata(provider: ProviderId) -> (&'static str, &'static str, &'static str, &'static str) {
    match provider {
        ProviderId::Aws => (
            "AWS API",
            "aws-sigv4",
            "https://docs.aws.amazon.com/IAM/latest/UserGuide/reference_sigv-create-signed-request.html",
            "Choose an API action, version, method and body from the service documentation. Query APIs such as EC2 use Action/Version; JSON APIs may require X-Amz-Target. Handle NextToken/Marker or other operation pagination explicitly. IAM remains authoritative; POST does not imply a write. Supply externally managed static or STS credentials; no automatic refresh or credential discovery.",
        ),
        ProviderId::Oci => (
            "OCI API",
            "oci-signature",
            "https://docs.oracle.com/en-us/iaas/Content/API/Concepts/signingrequests.htm",
            "Choose the operation, method, parameters and pagination from the relevant OCI service API. Use opc-next-page as documented and repeat target checks for each page. Signing uses the selected project's OC1 region and RSA API key; IAM remains authoritative.",
        ),
        ProviderId::Github => (
            "GitHub REST",
            "bearer",
            "https://docs.github.com/en/rest/using-the-rest-api/getting-started-with-the-rest-api",
            "Use an existing appropriately scoped token. Preserve explicit Accept and X-GitHub-Api-Version headers. Default version is 2022-11-28; follow operation documentation and Link pagination yourself. GitHub App private keys are not access tokens. Enterprise and download/upload domains are not implicitly authorized.",
        ),
        ProviderId::Cloudflare => (
            "Cloudflare control API",
            "bearer",
            "https://developers.cloudflare.com/fundamentals/api/how-to/make-api-calls/",
            "Use an appropriately scoped API token and resolve account/zone identifiers through documented APIs. Handle result_info or operation-specific cursors. Legacy global keys and Origin CA keys need explicitly configured custom headers and are not guessed as Bearer tokens.",
        ),
        ProviderId::DigitalOcean => (
            "DigitalOcean control API",
            "bearer",
            "https://docs.digitalocean.com/reference/api/reference/",
            "Use an appropriately scoped PAT or existing OAuth access token. Read links.pages and page/per_page per operation. Spaces S3 and Inference APIs are separate services and are not covered by this control-plane token adapter.",
        ),
        ProviderId::Hetzner => (
            "Hetzner Cloud",
            "bearer",
            "https://docs.hetzner.cloud/reference/cloud",
            "Use an existing project token with suitable Read or Read & Write permissions. Read meta.pagination.next_page and operation limits; do not assume all services use Link pagination. Robot and Storage Boxes are separate APIs.",
        ),
        ProviderId::Azure => (
            "Azure Resource Manager",
            "external-oauth",
            "https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow",
            "Requires tenant/application credentials or another supported identity flow, correct resource audience and token lifecycle. API versions and nextLink are operation-specific. Do not send client secrets or service-account documents as Bearer tokens.",
        ),
        ProviderId::Gcp => (
            "Google Cloud",
            "external-oauth",
            "https://developers.google.com/identity/protocols/oauth2/service-account",
            "Service-account JSON/private keys normally need JWT exchange for a scoped OAuth access token. Compute needs project/zone and pageToken handling. Token exchange, ADC and refresh are not implemented.",
        ),
        ProviderId::Alibaba => (
            "Alibaba ECS",
            "external-acs3",
            "https://www.alibabacloud.com/help/en/sdk/product-overview/v3-request-structure-and-signature",
            "Requires ACS3 AccessKey signing, action/version/date/nonce/content digest and optional STS security token. OSS/SLS and Model Studio have different authentication rules.",
        ),
        ProviderId::Tencent => (
            "Tencent Cloud API 3.0",
            "external-tc3",
            "https://cloud.tencent.com/document/product/213/30654",
            "Requires TC3 SecretId/SecretKey signing, X-TC-Action/Version/Region/Timestamp and optional temporary token. COS and older APIs have separate rules.",
        ),
        ProviderId::Huawei => (
            "Huawei Cloud",
            "external-huawei",
            "https://support.huaweicloud.com/eu/devg-apisign/api-sign-algorithm-001.html",
            "Requires SDK-HMAC-SHA256 AK/SK or a suitable IAM X-Auth-Token with project/domain scope. Not ordinary Authorization Bearer.",
        ),
        ProviderId::Ovh => (
            "OVHcloud",
            "external-ovh",
            "https://docs.ovhcloud.com/en/guides/manage-and-operate/api/first-steps",
            "Requires application key/secret, consumer key, timestamp and the OVH request signature. OpenStack APIs have a separate identity flow.",
        ),
    }
}

fn capability(endpoint: Endpoint) -> Result<ServiceCapability> {
    let service_documentation = match &endpoint {
        Endpoint::Aws(endpoint) => Some(match endpoint.signing_service {
            "ec2" => "https://docs.aws.amazon.com/AWSEC2/latest/APIReference/",
            "sts" => "https://docs.aws.amazon.com/STS/latest/APIReference/",
            "ssm" => "https://docs.aws.amazon.com/systems-manager/latest/APIReference/",
            "secretsmanager" => "https://docs.aws.amazon.com/secretsmanager/latest/apireference/",
            "logs" => "https://docs.aws.amazon.com/AmazonCloudWatchLogs/latest/APIReference/",
            "monitoring" => "https://docs.aws.amazon.com/AmazonCloudWatch/latest/APIReference/",
            "elasticloadbalancing" => {
                "https://docs.aws.amazon.com/elasticloadbalancing/latest/APIReference/"
            }
            "kms" => "https://docs.aws.amazon.com/kms/latest/APIReference/",
            "dynamodb" => "https://docs.aws.amazon.com/amazondynamodb/latest/APIReference/",
            "iam" => "https://docs.aws.amazon.com/IAM/latest/APIReference/",
            _ => "https://docs.aws.amazon.com/general/latest/gr/rande.html",
        }),
        Endpoint::Oci { .. } => Some("https://docs.oracle.com/en-us/iaas/api/"),
        _ => None,
    };
    let (provider, region, service, authentication, unsupported) = match endpoint {
        Endpoint::Generic => bail!(
            "URL is not a recognized native API service; use explicit generic HTTP configuration"
        ),
        Endpoint::Aws(endpoint) => (
            ProviderId::Aws,
            Some(endpoint.region),
            Some(endpoint.service),
            None,
            None,
        ),
        Endpoint::Oci { region } => (ProviderId::Oci, Some(region), None, None, None),
        Endpoint::Token(provider) => (provider, None, None, None, None),
        Endpoint::Unsupported {
            provider,
            service,
            authentication,
            reason,
        } => (
            provider,
            None,
            Some(service),
            Some(authentication),
            Some(reason),
        ),
    };
    let (name, auth, docs, instructions) = metadata(provider);
    let mut result = ServiceCapability {
        provider, service: service.unwrap_or(name).into(), authentication: authentication.unwrap_or(auth).into(),
        status: ConfigurationStatus::NeedsConfiguration, missing_fields: vec!["credentials".into()], region,
        documentation_urls: vec![docs.into()], instructions: instructions.into(),
        limitations: vec!["One request per call; no automatic pagination, privilege expansion, deployment or credential refresh".into(), "Native requests require HTTPS:443, do not use proxies, and do not follow redirects".into()],
    };
    if let Some(documentation) = service_documentation {
        result.documentation_urls.push(documentation.into());
    }
    if unsupported.is_some() && provider == ProviderId::DigitalOcean {
        result.documentation_urls =
            vec!["https://docs.digitalocean.com/reference/api/spaces/".into()];
        result.instructions = "Spaces uses its own S3-compatible API, region/space endpoint and signing key pair. The DigitalOcean control-plane Bearer token adapter does not implement this service.".into();
    }
    if unsupported.is_some() && provider == ProviderId::Hetzner {
        result.documentation_urls = vec!["https://docs.hetzner.cloud/".into()];
        result.instructions = "Select the exact Hetzner Robot, Storage Boxes or legacy DNS API and its documented credentials. Cloud API tokens and rules are not automatically applicable to these other services.".into();
    }
    if let Some(reason) = unsupported {
        result.status = ConfigurationStatus::Unsupported;
        result.missing_fields.clear();
        result.limitations.push(reason.into());
    }
    Ok(result)
}

pub fn describe_url(raw: &str) -> Result<ServiceCapability> {
    let url = Url::parse(raw).map_err(|_| anyhow::anyhow!("Invalid API service URL"))?;
    capability(identify(&url)?)
}

fn missing_roles(
    item: &StoredConnection,
    secrets: &SecretBundle,
    roles: &[(&str, Option<Uuid>)],
) -> Vec<String> {
    roles
        .iter()
        .filter(|(_, id)| value(item, secrets, *id).is_none())
        .map(|(name, _)| (*name).into())
        .collect()
}

pub fn describe_item(
    item: &StoredConnection,
    secrets: &SecretBundle,
) -> Result<Option<ServiceCapability>> {
    let endpoint = match saved_endpoint(item, secrets) {
        Ok(endpoint) => endpoint,
        Err(_) => {
            let provider = match item.http_auth_type.as_str() {
                crate::aws::AUTH_MODE => Some(ProviderId::Aws),
                crate::oci::AUTH_MODE => Some(ProviderId::Oci),
                _ => Url::parse(&item.base_url)
                    .ok()
                    .and_then(|url| match url.host_str() {
                        Some("api.github.com") => Some(ProviderId::Github),
                        Some("api.cloudflare.com") => Some(ProviderId::Cloudflare),
                        Some("api.digitalocean.com") => Some(ProviderId::DigitalOcean),
                        Some("api.hetzner.cloud") => Some(ProviderId::Hetzner),
                        _ => None,
                    }),
            };
            let Some(provider) = provider else {
                return Ok(None);
            };
            let mut result = capability(Endpoint::Token(provider))?;
            result.missing_fields = vec!["validServiceUrl".into()];
            return Ok(Some(result));
        }
    };
    if let Endpoint::Unsupported { provider, .. } = &endpoint {
        if external_oauth(*provider) && item.http_auth_type == "bearer" {
            let mut result = capability(endpoint)?;
            result.authentication = "externally-managed-oauth-bearer".into();
            result.missing_fields = if external_access_token(secrets).is_ok() {
                Vec::new()
            } else {
                vec!["apiCredentialAccessToken".into()]
            };
            if !item.enabled {
                result.missing_fields.push("enabled".into());
            }
            if item.certificate_authority.is_some() {
                result.missing_fields.push("notCertificateAuthority".into());
            }
            result.status = if result.missing_fields.is_empty() {
                ConfigurationStatus::Ready
            } else {
                ConfigurationStatus::NeedsConfiguration
            };
            result.limitations.push("Only an existing externally obtained access token is used; token acquisition, refresh and identity federation are not implemented".into());
            return Ok(Some(result));
        }
        return capability(endpoint).map(Some);
    }
    if item.http_auth_type == crate::aws::AUTH_MODE && !matches!(endpoint, Endpoint::Aws(_)) {
        let mut result = capability(Endpoint::Token(ProviderId::Aws))?;
        result.missing_fields = vec!["supportedAwsServiceUrl".into()];
        return Ok(Some(result));
    }
    if item.http_auth_type == crate::oci::AUTH_MODE && !matches!(endpoint, Endpoint::Oci { .. }) {
        let mut result = capability(Endpoint::Token(ProviderId::Oci))?;
        result.missing_fields = vec!["supportedOciServiceUrl".into()];
        return Ok(Some(result));
    }
    if matches!(endpoint, Endpoint::Generic) {
        return Ok(None);
    }
    let mut result = capability(endpoint)?;
    if matches!(
        result.provider,
        ProviderId::Github
            | ProviderId::Cloudflare
            | ProviderId::DigitalOcean
            | ProviderId::Hetzner
    ) && matches!(item.http_auth_type.as_str(), "apiKey" | "custom")
    {
        result.authentication = item.http_auth_type.clone();
    }
    if result.status == ConfigurationStatus::Unsupported {
        return Ok(Some(result));
    }
    result.missing_fields = match result.provider {
        ProviderId::Aws => {
            let mut missing = match item.aws_bindings.as_ref() {
                Some(binding) => missing_roles(
                    item,
                    secrets,
                    &[
                        ("serviceUrl", binding.service_url),
                        ("region", binding.region),
                        ("accessKeyId", binding.access_key_id),
                        ("secretAccessKey", binding.secret_access_key),
                    ],
                ),
                None => vec!["awsBindings".into()],
            };
            if item.http_auth_type != crate::aws::AUTH_MODE {
                missing.push("httpAuthType:aws-sigv4".into());
            }
            if let Some(binding) = &item.aws_bindings {
                if value(item, secrets, binding.access_key_id)
                    .is_some_and(|key| key.starts_with("ASIA"))
                    && value(item, secrets, binding.session_token).is_none()
                {
                    missing.push("sessionToken".into());
                }
            }
            result.region = item
                .aws_bindings
                .as_ref()
                .filter(|binding| visible_module(item, binding.region))
                .and_then(|_| crate::aws::configured_region(item, secrets).ok());
            if missing.is_empty() && crate::aws::Credential::from_modules(item, secrets).is_err() {
                missing.push("validAwsConfiguration".into());
            }
            missing
        }
        ProviderId::Oci => {
            let mut missing = match item.oci_bindings.as_ref() {
                Some(binding) => missing_roles(
                    item,
                    secrets,
                    &[
                        ("serviceUrl", binding.service_url),
                        ("region", binding.region),
                        ("tenancyOcid", binding.tenancy_ocid),
                        ("userOcid", binding.user_ocid),
                        ("fingerprint", binding.fingerprint),
                        ("privateKey", binding.private_key),
                    ],
                ),
                None => vec!["ociBindings".into()],
            };
            if item.http_auth_type != crate::oci::AUTH_MODE {
                missing.push("httpAuthType:oci-signature".into());
            }
            result.region = item
                .oci_bindings
                .as_ref()
                .filter(|binding| visible_module(item, binding.region))
                .and_then(|binding| value(item, secrets, binding.region))
                .filter(|region| crate::oci::REGIONS.contains(region))
                .map(str::to_owned);
            if missing.is_empty() && crate::oci::Credential::from_modules(item, secrets).is_err() {
                missing.push("validOciConfiguration".into());
            }
            missing
        }
        _ => match item.http_auth_type.as_str() {
            "bearer" if secrets.get("apiCredential").is_some() => Vec::new(),
            "apiKey" if !item.auth_header.is_empty() && secrets.get("apiCredential").is_some() => {
                Vec::new()
            }
            "custom"
                if !item.api_auth_headers.is_empty()
                    && item
                        .api_auth_headers
                        .iter()
                        .all(|h| secrets.get(&h.secret_name).is_some()) =>
            {
                Vec::new()
            }
            "bearer" | "apiKey" | "custom" => vec!["configuredAuthentication".into()],
            _ => vec!["httpAuthType".into()],
        },
    };
    if !item.enabled {
        result.missing_fields.push("enabled".into());
    }
    if item.certificate_authority.is_some() {
        result.missing_fields.push("notCertificateAuthority".into());
    }
    if result.missing_fields.is_empty() {
        result.status = ConfigurationStatus::Ready;
    }
    Ok(Some(result))
}

#[cfg(test)]
#[path = "api_provider_tests.rs"]
mod tests;
