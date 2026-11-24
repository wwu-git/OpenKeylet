use serde::Serialize;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiProfile {
    pub category: &'static str,
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub auth_type: &'static str,
    pub auth_header: &'static str,
    pub auth_location: &'static str,
    pub auth_prefix: &'static str,
}

/// Address templates for services that need signing or additional configuration.
/// These entries are not eligible for URL or authentication inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiAddressTemplate {
    pub category: &'static str,
    pub auth_mode: &'static str,
    pub display_name: &'static str,
    pub url_template: &'static str,
    pub requirements: &'static str,
}

pub const API_ADDRESS_TEMPLATES: &[ApiAddressTemplate] = &[
    // OCI commercial-cloud endpoints.
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "oci-signature",
        display_name: "OCI Compute / Networking",
        url_template: "https://iaas.<region>.oraclecloud.com/20160918/",
        requirements: "OCI request signing",
    },
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "oci-signature",
        display_name: "OCI Object Storage",
        url_template: "https://objectstorage.<region>.oraclecloud.com/",
        requirements: "OCI request signing",
    },
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "external",
        display_name: "AWS EC2",
        url_template: "https://ec2.<region>.amazonaws.com/",
        requirements: "AWS SigV4 request signing",
    },
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "external",
        display_name: "Alibaba Cloud ECS",
        url_template: "https://ecs.<region>.aliyuncs.com/",
        requirements: "Choose a supported ECS endpoint; ACS3 request signing",
    },
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "external",
        display_name: "Tencent Cloud CVM",
        url_template: "https://cvm.tencentcloudapi.com/",
        requirements: "TC3-HMAC-SHA256 request signing",
    },
    ApiAddressTemplate {
        category: "cloud",
        auth_mode: "external",
        display_name: "Huawei Cloud ECS",
        url_template: "https://ecs.<region>.myhuaweicloud.com/",
        requirements: "Choose a supported ECS endpoint; AK/SK request signing or an IAM X-Auth-Token",
    },
    ApiAddressTemplate {
        category: "developer",
        auth_mode: "bearer",
        display_name: "Notion",
        url_template: "https://api.notion.com/v1/",
        requirements: "Bearer token and a Notion-Version header",
    },
    ApiAddressTemplate {
        category: "ai",
        auth_mode: "bearer",
        display_name: "Alibaba Cloud Model Studio (Beijing pay-as-you-go)",
        url_template: "https://<workspace-id>.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/",
        requirements: "Bearer API key for the selected workspace and plan",
    },
    ApiAddressTemplate {
        category: "ai",
        auth_mode: "bearer",
        display_name: "Alibaba Cloud Model Studio (Singapore pay-as-you-go)",
        url_template: "https://<workspace-id>.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1/",
        requirements: "Bearer API key for the selected workspace and plan",
    },
    ApiAddressTemplate {
        category: "ai",
        auth_mode: "bearer",
        display_name: "Alibaba Cloud Model Studio (Beijing Token Plan)",
        url_template: "https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1/",
        requirements: "Token Plan bearer API key",
    },
    ApiAddressTemplate {
        category: "ai",
        auth_mode: "bearer",
        display_name: "Alibaba Cloud Model Studio (Coding Plan)",
        url_template: "https://coding.dashscope.aliyuncs.com/v1/",
        requirements: "Coding Plan bearer API key",
    },
];

const UNKNOWN: ApiProfile = ApiProfile {
    category: "developer",
    display_name: "API",
    default_base_url: "",
    auth_type: "bearer",
    auth_header: "Authorization",
    auth_location: "header",
    auth_prefix: "Bearer",
};

type ApiProfileEntry = (
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
    ApiProfile,
);

const PROFILES: &[ApiProfileEntry] = &[
    (
        &["api.stripe.com"],
        &["stripe"],
        &["sk_live_", "sk_test_", "rk_live_", "rk_test_"],
        ApiProfile {
            category: "payments",
            display_name: "Stripe",
            default_base_url: "https://api.stripe.com/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["api.anthropic.com"],
        &["anthropic", "claude"],
        &["sk-ant-"],
        ApiProfile {
            category: "ai",
            display_name: "Anthropic",
            default_base_url: "https://api.anthropic.com/v1/",
            auth_type: "apiKey",
            auth_header: "x-api-key",
            auth_location: "header",
            auth_prefix: "",
        },
    ),
    (
        &["openrouter.ai"],
        &["openrouter"],
        &["sk-or-v1-"],
        ApiProfile {
            category: "ai",
            display_name: "OpenRouter",
            default_base_url: "https://openrouter.ai/api/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["api.groq.com"],
        &["groq"],
        &["gsk_"],
        ApiProfile {
            category: "ai",
            display_name: "Groq",
            default_base_url: "https://api.groq.com/openai/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["api.openai.com"],
        &["openai", "chatgpt api"],
        &["sk-proj-", "sk-svcacct-"],
        ApiProfile {
            category: "ai",
            display_name: "OpenAI",
            default_base_url: "https://api.openai.com/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["api.github.com"],
        &["github"],
        &["github_pat_", "ghp_", "gho_", "ghu_", "ghs_"],
        ApiProfile {
            category: "developer",
            display_name: "GitHub",
            default_base_url: "https://api.github.com/",
            ..UNKNOWN
        },
    ),
    (
        &["api.cloudflare.com"],
        &["cloudflare"],
        &["cfut_"],
        ApiProfile {
            category: "cloud",
            display_name: "Cloudflare",
            default_base_url: "https://api.cloudflare.com/client/v4/",
            ..UNKNOWN
        },
    ),
    (
        &["generativelanguage.googleapis.com"],
        &["gemini", "google ai"],
        &["AIza"],
        ApiProfile {
            category: "ai",
            display_name: "Google Gemini",
            default_base_url: "https://generativelanguage.googleapis.com/v1beta/",
            auth_type: "apiKey",
            auth_header: "key",
            auth_location: "query",
            auth_prefix: "",
        },
    ),
    (
        &["api.deepseek.com"],
        &["deepseek"],
        &[],
        ApiProfile {
            category: "ai",
            display_name: "DeepSeek",
            default_base_url: "https://api.deepseek.com/",
            ..UNKNOWN
        },
    ),
    (
        &["open.bigmodel.cn"],
        &["zhipu", "智谱", "bigmodel"],
        &[],
        ApiProfile {
            category: "ai",
            display_name: "Zhipu GLM",
            default_base_url: "https://open.bigmodel.cn/api/paas/v4/",
            ..UNKNOWN
        },
    ),
    (
        &["api.mistral.ai"],
        &["mistral"],
        &[],
        ApiProfile {
            category: "ai",
            display_name: "Mistral",
            default_base_url: "https://api.mistral.ai/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["api.x.ai"],
        &["xai", "grok"],
        &[],
        ApiProfile {
            category: "ai",
            display_name: "xAI / Grok",
            default_base_url: "https://api.x.ai/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["gitlab.com"],
        &["gitlab"],
        &[],
        ApiProfile {
            category: "developer",
            display_name: "GitLab.com",
            default_base_url: "https://gitlab.com/api/v4/",
            auth_type: "apiKey",
            auth_header: "PRIVATE-TOKEN",
            auth_prefix: "",
            ..UNKNOWN
        },
    ),
    (
        &["slack.com"],
        &["slack"],
        &[],
        ApiProfile {
            category: "developer",
            display_name: "Slack",
            default_base_url: "https://slack.com/api/",
            ..UNKNOWN
        },
    ),
    (
        &["api.digitalocean.com"],
        &["digitalocean", "digital ocean"],
        &[],
        ApiProfile {
            category: "cloud",
            display_name: "DigitalOcean",
            default_base_url: "https://api.digitalocean.com/v2/",
            ..UNKNOWN
        },
    ),
    (
        &["api.hetzner.cloud"],
        &["hetzner"],
        &[],
        ApiProfile {
            category: "cloud",
            display_name: "Hetzner Cloud",
            default_base_url: "https://api.hetzner.cloud/v1/",
            ..UNKNOWN
        },
    ),
    (
        &["management.azure.com"],
        &["azure resource manager", "azure arm"],
        &[],
        ApiProfile {
            category: "cloud",
            display_name: "Azure Resource Manager",
            default_base_url: "https://management.azure.com/",
            ..UNKNOWN
        },
    ),
    (
        &["compute.googleapis.com"],
        &["google cloud compute", "compute engine"],
        &[],
        ApiProfile {
            category: "cloud",
            display_name: "Google Cloud Compute Engine",
            default_base_url: "https://compute.googleapis.com/compute/v1/",
            ..UNKNOWN
        },
    ),
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiService {
    pub id: String,
    pub name: &'static str,
    pub category: &'static str,
    pub url_template: &'static str,
    pub parameters: Vec<&'static str>,
    pub requirements: String,
    pub auth_mode: &'static str,
}

pub fn service_catalog() -> Vec<ApiService> {
    let profiles = PROFILES.iter().map(|(_, _, _, profile)| ApiService {
        id: service_id("profile", profile.display_name),
        name: profile.display_name,
        category: profile.category,
        url_template: profile.default_base_url,
        parameters: Vec::new(),
        requirements: match profile.auth_type {
            "apiKey" if profile.auth_location == "query" => {
                format!("API key in the {} query parameter", profile.auth_header)
            }
            "apiKey" => format!("API key in the {} request header", profile.auth_header),
            _ => "Bearer token".to_owned(),
        },
        auth_mode: profile.auth_type,
    });
    let templates = API_ADDRESS_TEMPLATES.iter().map(|template| ApiService {
        id: service_id("template", template.display_name),
        name: template.display_name,
        category: template.category,
        url_template: template.url_template,
        parameters: template_parameters(template.url_template),
        requirements: if template.auth_mode == "external" {
            format!(
                "Address only; native request signing is not implemented. {}",
                template.requirements
            )
        } else {
            template.requirements.to_owned()
        },
        auth_mode: template.auth_mode,
    });
    profiles.chain(templates).collect()
}

fn service_id(source: &str, name: &str) -> String {
    let slug = name
        .split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join("-");
    format!("{source}-{slug}")
}

fn template_parameters(template: &'static str) -> Vec<&'static str> {
    let mut parameters = Vec::new();
    for part in template.split('<').skip(1) {
        if let Some((parameter, _)) = part.split_once('>') {
            if !parameters.contains(&parameter) {
                parameters.push(parameter);
            }
        }
    }
    parameters
}

pub fn infer(name: &str, base_url: &str, secret: &str) -> ApiProfile {
    if !base_url.trim().is_empty() {
        return normalize_base_url(base_url, "")
            .ok()
            .and_then(|value| Url::parse(&value).ok())
            .and_then(|url| {
                let host = url.host_str()?;
                PROFILES
                    .iter()
                    .find(|(hosts, _, _, _)| hosts.contains(&host))
                    .map(|(_, _, _, profile)| *profile)
            })
            .unwrap_or(UNKNOWN);
    }
    let name = name.to_ascii_lowercase();
    for (_, aliases, _, profile) in PROFILES {
        if aliases.iter().any(|alias| name.contains(alias)) {
            return *profile;
        }
    }
    for (_, _, prefixes, profile) in PROFILES {
        if prefixes.iter().any(|prefix| secret.starts_with(prefix)) {
            return *profile;
        }
    }
    UNKNOWN
}

pub fn normalize_base_url(raw: &str, fallback: &str) -> anyhow::Result<String> {
    let raw = if raw.trim().is_empty() {
        fallback.trim()
    } else {
        raw.trim()
    };
    if raw.is_empty() {
        return Ok(String::new());
    }
    let candidate = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("https://{raw}")
    };
    let url = Url::parse(&candidate).map_err(|_| anyhow::anyhow!("Invalid API URL"))?;
    Ok(url.to_string())
}

pub fn fallback_name(base_url: &str, profile: ApiProfile) -> String {
    if profile != UNKNOWN {
        return profile.display_name.to_owned();
    }
    Url::parse(base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| UNKNOWN.display_name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_picker_returns_unique_complete_addresses_without_inference_material() {
        let catalog = service_catalog();
        let mut ids = std::collections::HashSet::new();
        for service in &catalog {
            assert!(
                ids.insert(&service.id),
                "duplicate picker ID: {}",
                service.id
            );
            assert!(["ai", "developer", "cloud", "payments"].contains(&service.category));
            let mut filled = service.url_template.to_owned();
            for parameter in &service.parameters {
                let value = match *parameter {
                    "region" => "ap-tokyo-1",
                    "workspace-id" => "example-workspace",
                    _ => panic!("unsupported picker parameter: {parameter}"),
                };
                filled = filled.replace(&format!("<{parameter}>"), value);
            }
            assert!(!filled.contains(['<', '>']));
            let url = Url::parse(&filled).unwrap();
            assert_eq!(url.scheme(), "https");
            assert!(url.host_str().is_some());
            assert!(url.username().is_empty() && url.password().is_none());
            assert!(!service.requirements.is_empty());
        }
        let json = serde_json::to_value(&catalog).unwrap();
        assert!(json[0].get("urlTemplate").is_some());
        assert!(json[0].get("authMode").is_some());
        let encoded = json.to_string();
        for prefix in ["ghp_", "github_pat_", "sk-ant-", "sk-proj-", "prefixes"] {
            assert!(!encoded.contains(prefix));
        }
        let github = catalog
            .iter()
            .find(|service| service.name == "GitHub")
            .unwrap();
        assert_eq!(github.url_template, "https://api.github.com/");
        assert_eq!(github.auth_mode, "bearer");
        assert!(github.parameters.is_empty());
    }

    #[test]
    fn picker_distinguishes_oci_signing_from_external_signing_templates() {
        let catalog = service_catalog();
        for name in ["OCI Compute / Networking", "OCI Object Storage"] {
            let service = catalog.iter().find(|service| service.name == name).unwrap();
            assert_eq!(service.auth_mode, "oci-signature");
            assert_eq!(service.parameters, ["region"]);
            assert_eq!(service.category, "cloud");
        }
        for name in [
            "AWS EC2",
            "Alibaba Cloud ECS",
            "Tencent Cloud CVM",
            "Huawei Cloud ECS",
        ] {
            let service = catalog.iter().find(|service| service.name == name).unwrap();
            assert_eq!(service.auth_mode, "external");
            assert!(service.requirements.contains("Address only"));
            assert!(service.requirements.contains("not implemented"));
            assert_eq!(infer(name, "", ""), UNKNOWN);
        }
        let notion = catalog
            .iter()
            .find(|service| service.name == "Notion")
            .unwrap();
        assert!(notion.requirements.contains("Notion-Version"));
        assert_eq!(notion.auth_mode, "bearer");
        let workspace = catalog
            .iter()
            .find(|service| service.name.contains("Singapore pay-as-you-go"))
            .unwrap();
        assert_eq!(workspace.parameters, ["workspace-id"]);
    }

    #[test]
    fn picker_parameters_are_ordered_and_repeated_names_are_requested_once() {
        assert_eq!(
            template_parameters("https://<region>.example/<workspace-id>/<region>"),
            ["region", "workspace-id"]
        );
        assert!(template_parameters("https://api.example/").is_empty());
    }

    #[test]
    fn infers_only_from_the_actual_host_when_a_url_is_given() {
        let gitlab = infer("GitLab", "gitlab.com/api/v4", "");
        assert_eq!(gitlab.auth_header, "PRIVATE-TOKEN");
        assert_eq!(gitlab.auth_prefix, "");
        assert_eq!(
            infer("GitLab", "https://api.deepseek.com/", "").display_name,
            "DeepSeek"
        );
        assert_eq!(
            infer(
                "GitLab",
                "https://gitlab.com.example.test/api/gitlab.com",
                "sk-ant-example"
            ),
            UNKNOWN
        );
        assert_eq!(
            infer("Hetzner project", "", "").default_base_url,
            "https://api.hetzner.cloud/v1/"
        );
    }

    #[test]
    fn address_templates_stay_out_of_auto_inference() {
        for entry in API_ADDRESS_TEMPLATES {
            let url = entry
                .url_template
                .replace("<region>", "ap-tokyo-1")
                .replace("<workspace-id>", "example-workspace");
            assert_eq!(Url::parse(&url).unwrap().scheme(), "https");
            assert_eq!(infer(entry.display_name, &url, ""), UNKNOWN);
            assert_eq!(infer(entry.display_name, "", "").default_base_url, "");
        }
    }

    #[test]
    fn detects_common_provider_and_adds_https() {
        let profile = infer("", "", "sk-ant-example");
        assert_eq!(profile.display_name, "Anthropic");
        assert_eq!(profile.auth_header, "x-api-key");
        assert_eq!(
            normalize_base_url("api.example.com/v1", "").unwrap(),
            "https://api.example.com/v1"
        );
    }

    #[test]
    fn unknown_provider_can_remain_addressless() {
        // A generic sk- prefix is shared by several providers and must not be
        // guessed as OpenAI without a name or URL.
        let profile = infer("", "", "sk-generic-secret");
        assert_eq!(profile, UNKNOWN);
        assert_eq!(
            normalize_base_url("", profile.default_base_url).unwrap(),
            ""
        );
        assert_eq!(fallback_name("", profile), "API");
    }
}
