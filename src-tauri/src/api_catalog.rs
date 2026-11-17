use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiProfile {
    pub display_name: &'static str,
    pub default_base_url: &'static str,
    pub auth_type: &'static str,
    pub auth_header: &'static str,
    pub auth_location: &'static str,
    pub auth_prefix: &'static str,
}

/// Address templates for services that require per-request signing.
/// These entries are not eligible for URL or authentication inference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedApiEndpoint {
    pub display_name: &'static str,
    pub url_template: &'static str,
    pub signing: &'static str,
}

// OCI commercial-cloud endpoints:
// https://docs.oracle.com/en-us/iaas/Content/API/Concepts/usingapi.htm
pub const SIGNED_API_ENDPOINTS: &[SignedApiEndpoint] = &[
    SignedApiEndpoint {
        display_name: "OCI Compute / Networking",
        url_template: "https://iaas.<region>.oraclecloud.com/20160918/",
        signing: "OCI request signing",
    },
    SignedApiEndpoint {
        display_name: "OCI Object Storage",
        url_template: "https://objectstorage.<region>.oraclecloud.com/",
        signing: "OCI request signing",
    },
];

const UNKNOWN: ApiProfile = ApiProfile {
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
            display_name: "Google Gemini",
            default_base_url: "https://generativelanguage.googleapis.com/v1beta/",
            auth_type: "apiKey",
            auth_header: "key",
            auth_location: "query",
            auth_prefix: "",
        },
    ),
];

pub fn infer(name: &str, base_url: &str, secret: &str) -> ApiProfile {
    let text = format!("{name} {base_url}").to_ascii_lowercase();
    for (hosts, aliases, _, profile) in PROFILES {
        if hosts.iter().any(|host| text.contains(host))
            || aliases.iter().any(|alias| text.contains(alias))
        {
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
    fn oci_templates_do_not_infer_bearer_profiles_or_default_urls() {
        for entry in SIGNED_API_ENDPOINTS {
            let url = entry.url_template.replace("<region>", "ap-tokyo-1");
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
