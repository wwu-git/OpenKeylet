use super::*;
use crate::{model::ConnectionInput, vault::Vault};
use serde_json::json;

fn endpoint(raw: &str) -> Result<Endpoint> {
    identify(&Url::parse(raw).unwrap())
}

fn item(raw: &str) -> (tempfile::TempDir, Vault, uuid::Uuid) {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    let input: ConnectionInput = serde_json::from_value(json!({
        "name":"Synthetic provider", "itemType":"api", "httpAuthType":"bearer",
        "modules":[{"kind":"url", "value":raw},{"kind":"apiCredential"}],
        "secrets":{"namedSecrets":{"apiCredential":"synthetic-provider-token"}}
    }))
    .unwrap();
    let saved = vault.save_connection(input).unwrap();
    (directory, vault, saved.id)
}

#[test]
fn identifies_exact_token_service_paths_without_brand_wildcards() {
    for (raw, provider) in [
        ("https://api.github.com/repos/a/b", ProviderId::Github),
        (
            "https://api.cloudflare.com/client/v4/zones",
            ProviderId::Cloudflare,
        ),
        (
            "https://api.digitalocean.com/v2/droplets",
            ProviderId::DigitalOcean,
        ),
        ("https://api.hetzner.cloud/v1/servers", ProviderId::Hetzner),
    ] {
        assert_eq!(endpoint(raw).unwrap(), Endpoint::Token(provider));
    }
    for raw in [
        "https://api.github.com.attacker.invalid/",
        "https://github.com/api",
        "https://other.api.hetzner.cloud/v1/servers",
    ] {
        assert_eq!(endpoint(raw).unwrap(), Endpoint::Generic);
    }
    for raw in [
        "https://api.digitalocean.com/v20/droplets",
        "https://api.cloudflare.com/client/v40/zones",
        "https://api.hetzner.cloud/v11",
    ] {
        assert!(endpoint(raw).is_err());
    }
    assert!(matches!(
        endpoint("https://test.nyc3.digitaloceanspaces.com/").unwrap(),
        Endpoint::Unsupported {
            provider: ProviderId::DigitalOcean,
            ..
        }
    ));
    assert!(matches!(
        endpoint("https://robot-ws.your-server.de/server").unwrap(),
        Endpoint::Unsupported {
            provider: ProviderId::Hetzner,
            ..
        }
    ));
}

#[test]
fn rejects_insecure_ambiguous_native_urls_and_aws_endpoint_variants() {
    for raw in [
        "http://api.github.com/",
        "https://api.github.com./",
        "https://ec2.us-east-1.amazonaws.com./",
        "https://api.github.com:444/",
        "https://api.github.com/#token",
        "https://user:pass@api.github.com/",
        "http://ec2.us-east-1.amazonaws.com/",
    ] {
        assert!(endpoint(raw).is_err(), "{raw}");
    }
    for raw in [
        "https://s3.us-east-1.amazonaws.com/",
        "https://bucket.s3.us-east-1.amazonaws.com/",
        "https://sts.amazonaws.com/",
        "https://ec2.cn-north-1.amazonaws.com.cn/",
        "https://ec2.us-gov-west-1.amazonaws.com/",
        "https://ec2-fips.us-east-1.amazonaws.com/",
        "https://ec2.us-east-1.api.aws/",
        "https://ec2.not-a-region.amazonaws.com/",
        "https://lambda.us-east-1.amazonaws.com/",
    ] {
        assert!(
            matches!(
                endpoint(raw).unwrap(),
                Endpoint::Unsupported {
                    provider: ProviderId::Aws,
                    ..
                }
            ),
            "{raw}"
        );
    }
    for (host, service) in AWS_SERVICES {
        let Endpoint::Aws(result) =
            endpoint(&format!("https://{host}.us-east-1.amazonaws.com/")).unwrap()
        else {
            panic!("supported AWS service")
        };
        assert_eq!(result.region, "us-east-1");
        assert_eq!(result.signing_service, *host);
        assert_eq!(result.service, *service);
    }
    let Endpoint::Aws(iam) = endpoint("https://iam.amazonaws.com/").unwrap() else {
        panic!("global IAM")
    };
    assert_eq!(iam.region, "us-east-1");
    assert_eq!(iam.signing_service, "iam");
}

#[test]
fn prevents_cross_provider_secret_use_and_signer_bearer_fallback() {
    let (_directory, vault, id) = item("https://api.github.com/");
    let connection = vault.get_connection(id).unwrap();
    assert!(matches!(
        execution_plan(
            &connection.stored,
            &connection.secrets,
            &Url::parse("https://api.github.com/repos/a/b").unwrap()
        )
        .unwrap(),
        ExecutionPlan::Token(ProviderId::Github)
    ));
    for raw in [
        "https://api.cloudflare.com/client/v4/",
        "https://api.github.com.attacker.invalid/",
        "https://127.0.0.1/",
        "https://ec2.us-east-1.amazonaws.com/",
        "https://iaas.us-ashburn-1.oraclecloud.com/",
    ] {
        assert!(
            execution_plan(
                &connection.stored,
                &connection.secrets,
                &Url::parse(raw).unwrap()
            )
            .is_err(),
            "{raw}"
        );
    }
    let (_directory, vault, id) = item("https://api.example.invalid/");
    let connection = vault.get_connection(id).unwrap();
    assert!(matches!(
        execution_plan(
            &connection.stored,
            &connection.secrets,
            &Url::parse("http://127.0.0.1:18888/").unwrap()
        )
        .unwrap(),
        ExecutionPlan::Generic
    ));
    assert!(
        execution_plan(
            &connection.stored,
            &connection.secrets,
            &Url::parse("https://ecs.cn-hangzhou.aliyuncs.com/").unwrap()
        )
        .is_err()
    );
}

#[test]
fn github_defaults_preserve_explicit_api_version_and_media_type() {
    let mut headers = HeaderMap::new();
    apply_defaults(ProviderId::Github, &mut headers).unwrap();
    assert_eq!(headers[ACCEPT], "application/vnd.github+json");
    assert_eq!(headers["x-github-api-version"], "2022-11-28");
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github.raw+json"),
    );
    headers.insert(
        "x-github-api-version",
        HeaderValue::from_static("2026-03-10"),
    );
    apply_defaults(ProviderId::Github, &mut headers).unwrap();
    assert_eq!(headers[ACCEPT], "application/vnd.github.raw+json");
    assert_eq!(headers["x-github-api-version"], "2026-03-10");
    let mut other = HeaderMap::new();
    apply_defaults(ProviderId::Cloudflare, &mut other).unwrap();
    assert!(other.is_empty());
}

#[test]
fn reports_known_unsupported_authentication_without_requesting_plaintext() {
    for raw in [
        "https://management.azure.com/",
        "https://compute.googleapis.com/compute/v1/",
        "https://ecs.cn-shanghai.aliyuncs.com/",
        "https://cvm.tencentcloudapi.com/",
        "https://ecs.cn-north-4.myhuaweicloud.com/",
        "https://api.eu.ovhcloud.com/1.0/",
    ] {
        let capability = describe_url(raw).unwrap();
        assert_eq!(capability.status, ConfigurationStatus::Unsupported);
        assert!(!capability.documentation_urls.is_empty());
    }
    let url_only = describe_url("https://ec2.us-east-1.amazonaws.com/").unwrap();
    assert_eq!(url_only.status, ConfigurationStatus::NeedsConfiguration);
    assert!(describe_url("https://api.example.invalid/").is_err());
    let (_directory, vault, id) = item("https://api.github.com/");
    let connection = vault.get_connection(id).unwrap();
    let description = describe_item(&connection.stored, &connection.secrets)
        .unwrap()
        .unwrap();
    assert_eq!(description.status, ConfigurationStatus::Ready);
    let output = serde_json::to_string(&description).unwrap();
    assert!(!output.contains("synthetic-provider-token"));
}

#[test]
fn aws_binding_visibility_readiness_and_region_scope_follow_saved_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().into()).unwrap();
    let ids = [
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    ];
    let input: ConnectionInput = serde_json::from_value(json!({
        "name":"Synthetic AWS", "itemType":"api", "httpAuthType":"aws-sigv4",
        "awsBindings":{"serviceUrl":ids[0],"accessKeyId":ids[1],"secretAccessKey":ids[2],"region":ids[3]},
        "modules":[{"id":ids[0],"kind":"url","value":"https://ec2.us-east-1.amazonaws.com/"},{"id":ids[1],"kind":"customSecret","name":"key","agentVisible":false},{"id":ids[2],"kind":"customSecret","name":"secret","agentVisible":false},{"id":ids[3],"kind":"customSecret","name":"region","agentVisible":false}],
        "secrets":{"namedSecrets":{"key":"AKIDEXAMPLE","secret":"synthetic-secret","region":"us-east-1"}}
    })).unwrap();
    let id = vault.save_connection(input).unwrap().id;
    let connection = vault.get_connection(id).unwrap();
    let info = describe_item(&connection.stored, &connection.secrets)
        .unwrap()
        .unwrap();
    assert_eq!(info.status, ConfigurationStatus::Ready);
    assert_eq!(info.region, None);
    let mut visible = connection.stored.clone();
    visible.modules[3].agent_visible = Some(true);
    assert_eq!(
        describe_item(&visible, &connection.secrets)
            .unwrap()
            .unwrap()
            .region
            .as_deref(),
        Some("us-east-1")
    );
    assert!(matches!(
        execution_plan(
            &visible,
            &connection.secrets,
            &Url::parse("https://ssm.us-east-1.amazonaws.com/").unwrap()
        )
        .unwrap(),
        ExecutionPlan::Aws(_)
    ));
    assert!(
        execution_plan(
            &visible,
            &connection.secrets,
            &Url::parse("https://ssm.us-west-2.amazonaws.com/").unwrap()
        )
        .is_err()
    );
    let mut mismatched = connection.secrets.clone();
    mismatched
        .named_secrets
        .insert("region".into(), "us-west-2".into());
    visible.modules[0].agent_visible = Some(false);
    let mismatch = describe_item(&visible, &mismatched).unwrap().unwrap();
    assert_eq!(mismatch.status, ConfigurationStatus::NeedsConfiguration);
    assert_eq!(mismatch.region.as_deref(), Some("us-west-2"));
    assert!(
        !serde_json::to_string(&mismatch)
            .unwrap()
            .contains("us-east-1")
    );
    let mut temporary = connection.secrets.clone();
    temporary
        .named_secrets
        .insert("key".into(), "ASIASYNTHETICEXAMPLE".into());
    let info = describe_item(&connection.stored, &temporary)
        .unwrap()
        .unwrap();
    assert!(info.missing_fields.contains(&"sessionToken".into()));
    let mut wrong = connection.stored.clone();
    wrong.aws_bindings.as_mut().unwrap().access_key_id = None;
    let info = describe_item(&wrong, &connection.secrets).unwrap().unwrap();
    assert_eq!(info.status, ConfigurationStatus::NeedsConfiguration);
    assert!(info.missing_fields.contains(&"accessKeyId".into()));
    let output = serde_json::to_string(&info).unwrap();
    assert!(
        !output.contains("synthetic-secret")
            && !output.contains("AKIDEXAMPLE")
            && !output.contains("us-east-1")
    );
}

#[test]
fn incomplete_native_items_do_not_break_capability_discovery() {
    let (_directory, vault, id) = item("https://api.github.com/");
    let connection = vault.get_connection(id).unwrap();
    let mut broken = connection.stored.clone();
    broken.base_url = "http://api.github.com/".into();
    let info = describe_item(&broken, &connection.secrets)
        .unwrap()
        .unwrap();
    assert_eq!(info.status, ConfigurationStatus::NeedsConfiguration);
    assert_eq!(info.missing_fields, ["validServiceUrl"]);
    broken.http_auth_type = "aws-sigv4".into();
    broken.base_url.clear();
    let info = describe_item(&broken, &connection.secrets)
        .unwrap()
        .unwrap();
    assert_eq!(info.provider, ProviderId::Aws);
    assert_eq!(info.status, ConfigurationStatus::NeedsConfiguration);
    assert_eq!(info.missing_fields, ["supportedAwsServiceUrl"]);
    broken.base_url = "https://s3.us-east-1.amazonaws.com/".into();
    assert_eq!(
        describe_item(&broken, &connection.secrets)
            .unwrap()
            .unwrap()
            .status,
        ConfigurationStatus::Unsupported
    );
}

#[test]
fn preserves_external_azure_and_gcp_access_tokens_without_implying_oauth_exchange() {
    for raw in [
        "https://management.azure.com/",
        "https://compute.googleapis.com/compute/v1/",
    ] {
        let (_directory, vault, id) = item(raw);
        let connection = vault.get_connection(id).unwrap();
        let info = describe_item(&connection.stored, &connection.secrets)
            .unwrap()
            .unwrap();
        assert_eq!(info.status, ConfigurationStatus::Ready);
        assert_eq!(info.authentication, "externally-managed-oauth-bearer");
        assert!(
            info.limitations
                .iter()
                .any(|limit| limit.contains("refresh"))
        );
        assert!(matches!(
            execution_plan(
                &connection.stored,
                &connection.secrets,
                &Url::parse(raw).unwrap()
            )
            .unwrap(),
            ExecutionPlan::Token(_)
        ));
        for other in [
            "https://attacker.invalid/",
            "https://api.github.com/",
            "https://cvm.tencentcloudapi.com/",
        ] {
            assert!(
                execution_plan(
                    &connection.stored,
                    &connection.secrets,
                    &Url::parse(other).unwrap()
                )
                .is_err()
            );
        }
        for bad in [
            "{\"private_key\":\"synthetic\"}",
            "-----BEGIN PRIVATE KEY-----\nsynthetic\n-----END PRIVATE KEY-----",
            "client secret with spaces",
        ] {
            let mut secrets = connection.secrets.clone();
            secrets
                .named_secrets
                .insert("apiCredential".into(), bad.into());
            assert!(
                execution_plan(&connection.stored, &secrets, &Url::parse(raw).unwrap()).is_err()
            );
            assert_eq!(
                describe_item(&connection.stored, &secrets)
                    .unwrap()
                    .unwrap()
                    .status,
                ConfigurationStatus::NeedsConfiguration
            );
        }
    }
}

#[test]
fn unsupported_services_do_not_inherit_brand_bearer_metadata_or_escape_to_generic() {
    let spaces = describe_url("https://test.nyc3.digitaloceanspaces.com/").unwrap();
    assert_eq!(spaces.service, "DigitalOcean Spaces");
    assert_eq!(spaces.authentication, "external-s3-signing");
    assert_eq!(
        spaces.documentation_urls,
        ["https://docs.digitalocean.com/reference/api/spaces/"]
    );
    assert!(
        !spaces
            .instructions
            .contains("Use an appropriately scoped PAT")
    );
    let robot = describe_url("https://robot-ws.your-server.de/").unwrap();
    assert_ne!(robot.authentication, "bearer");
    assert_ne!(robot.service, "Hetzner Cloud");
    for raw in [
        "https://ec2.us-east-1.amazonaws.com/",
        "https://iaas.us-ashburn-1.oraclecloud.com/",
        "https://cvm.tencentcloudapi.com/",
    ] {
        let (_directory, vault, id) = item(raw);
        let connection = vault.get_connection(id).unwrap();
        assert!(
            execution_plan(
                &connection.stored,
                &connection.secrets,
                &Url::parse("https://attacker.invalid/").unwrap()
            )
            .is_err()
        );
    }
}
