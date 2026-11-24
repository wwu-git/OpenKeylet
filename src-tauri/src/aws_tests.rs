use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

fn credential(region: &str, token: Option<&str>) -> Credential {
    Credential {
        config: Config {
            region: region.into(),
        },
        access_key_id: Zeroizing::new("AKIDEXAMPLE".into()),
        secret_access_key: Zeroizing::new("synthetic-secret-key-for-oracle".into()),
        session_token: token.map(|value| Zeroizing::new(value.into())),
    }
}
fn endpoint(url: &str) -> AwsEndpoint {
    match crate::api_provider::identify(&Url::parse(url).unwrap()).unwrap() {
        Endpoint::Aws(value) => value,
        _ => panic!("fixture endpoint must be AWS"),
    }
}
fn request() -> Request {
    Request::new(
        reqwest::Method::GET,
        Url::parse("https://ec2.us-east-1.amazonaws.com/").unwrap(),
    )
}

#[test]
fn published_iam_sigv4_reference_signature_matches() {
    let mut headers = HeaderMap::new();
    headers.insert(
        reqwest::header::HOST,
        HeaderValue::from_static("iam.amazonaws.com"),
    );
    headers.insert(
        reqwest::header::CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded; charset=utf-8"),
    );
    headers.insert(
        HeaderName::from_static("x-amz-date"),
        HeaderValue::from_static("20150830T123600Z"),
    );
    let (headers, names) = canonical_headers(&headers).unwrap();
    let canonical = format!(
        "GET\n/\nAction=ListUsers&Version=2010-05-08\n{headers}\n{names}\n{}",
        hash(b"")
    );
    assert_eq!(
        signature(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830T123600Z",
            "us-east-1",
            "iam",
            &canonical
        ),
        "5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
    );
}

#[test]
fn complete_prepared_requests_match_independent_aws_sdk_fixtures() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../tests/fixtures/aws-sigv4-botocore.json")).unwrap();
    for case in fixtures["fixtures"].as_array().unwrap() {
        let region = case["region"].as_str().unwrap();
        let token = case["sessionToken"].as_str();
        let credential = credential(region, token);
        let mut request = Request::new(
            case["method"].as_str().unwrap().parse().unwrap(),
            Url::parse(case["url"].as_str().unwrap()).unwrap(),
        );
        for (name, value) in case["headers"].as_object().unwrap() {
            if name.eq_ignore_ascii_case("x-amz-content-sha256") {
                continue;
            }
            request.headers_mut().insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value.as_str().unwrap()).unwrap(),
            );
        }
        let body = STANDARD
            .decode(case["bodyBase64"].as_str().unwrap())
            .unwrap();
        *request.body_mut() = Some(body.clone().into());
        let endpoint = endpoint(case["url"].as_str().unwrap());
        let (prepared, authorization) = prepare(
            &credential,
            &endpoint,
            request,
            case["date"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            prepared.url().as_str(),
            case["expected"]["url"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(
            authorization.to_str().unwrap(),
            case["expected"]["authorization"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert_eq!(prepared.body().unwrap().as_bytes().unwrap(), body);
        assert!(prepared.headers()[reqwest::header::AUTHORIZATION].is_sensitive());
        if token.is_some() {
            assert!(prepared.headers()["x-amz-security-token"].is_sensitive());
        }
    }
}

#[test]
fn signing_rejects_reserved_headers_presign_streaming_and_wrong_scope() {
    let credential = credential("us-east-1", None);
    let ep = endpoint("https://ec2.us-east-1.amazonaws.com/");
    for name in RESERVED_HEADERS {
        let mut request = request();
        request.headers_mut().insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_static("caller-value"),
        );
        assert!(validate_request(&request).is_err(), "{name}");
        assert!(prepare(&credential, &ep, request, "20261124T123456Z").is_err());
    }
    for target in [
        "https://ec2.us-east-1.amazonaws.com/?X-Amz-Signature=caller",
        "https://ec2.us-east-1.amazonaws.com/?X-Amz-Credential=caller",
        "https://ec2.us-east-1.amazonaws.com/?SignatureVersion=4",
        "https://s3.us-east-1.amazonaws.com/",
        "https://ec2.us-east-1.amazonaws.com.evil.invalid/",
        "http://ec2.us-east-1.amazonaws.com/",
        "https://ec2.us-east-1.amazonaws.com:8443/",
        "https://user@ec2.us-east-1.amazonaws.com/",
        "https://ec2.us-east-1.amazonaws.com/#fragment",
        "https://ec2.eu-west-1.amazonaws.com/",
    ] {
        let request = Request::new(reqwest::Method::GET, Url::parse(target).unwrap());
        assert!(
            prepare(&credential, &ep, request, "20261124T123456Z").is_err(),
            "{target}"
        );
    }
    for target in [
        "https://ec2.us-east-1.amazonaws.com/?bad=%FF",
        "https://ec2.us-east-1.amazonaws.com/?bad=%GG",
        "https://ec2.us-east-1.amazonaws.com/?bad=%",
        "https://ec2.us-east-1.amazonaws.com/?bad=%2",
        "https://ec2.us-east-1.amazonaws.com/?bad=%C0%AF",
        "https://ec2.us-east-1.amazonaws.com/?%FF=value",
    ] {
        let request = Request::new(reqwest::Method::GET, Url::parse(target).unwrap());
        assert!(validate_request(&request).is_err(), "{target}");
    }
    let mut streaming = request();
    streaming.headers_mut().insert(
        reqwest::header::CONTENT_ENCODING,
        HeaderValue::from_static("aws-chunked"),
    );
    assert!(validate_request(&streaming).is_err());
    assert!(prepare(&credential, &ep, request(), "bad-date").is_err());
    assert!(prepare(&credential, &ep, request(), "20261324T123456Z").is_err());
    let wrong_service = AwsEndpoint {
        signing_service: "sts",
        ..ep
    };
    assert!(prepare(&credential, &wrong_service, request(), "20261124T123456Z").is_err());
}

fn module_fixture(access_key: &str, token: &str) -> (StoredConnection, SecretBundle) {
    let url = Uuid::new_v4();
    let key = Uuid::new_v4();
    let secret = Uuid::new_v4();
    let token_id = Uuid::new_v4();
    let region = Uuid::new_v4();
    let input: crate::model::ConnectionInput = serde_json::from_value(json!({
        "name":"Synthetic AWS", "itemType":"api", "httpAuthType":"aws-sigv4",
        "awsBindings":{"serviceUrl":url,"accessKeyId":key,"secretAccessKey":secret,"sessionToken":token_id,"region":region},
        "modules":[{"id":url,"kind":"url","value":"https://ec2.us-east-1.amazonaws.com/"},{"id":key,"kind":"customSecret","name":"key id"},{"id":secret,"kind":"customSecret","name":"secret key"},{"id":token_id,"kind":"customSecret","name":"session token"},{"id":region,"kind":"customSecret","name":"region"}],
        "secrets":{"namedSecrets":{"key id":access_key,"secret key":"synthetic-secret","session token":token,"region":"us-east-1"}}
    })).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let vault = crate::vault::Vault::open(dir.path().into()).unwrap();
    let saved = vault.save_connection(input).unwrap();
    let connection = vault.get_connection(saved.id).unwrap();
    (connection.stored, connection.secrets)
}

#[test]
fn credential_bindings_survive_names_and_require_temporary_token() {
    let (mut item, mut secrets) = module_fixture("AKIDEXAMPLE", "");
    assert!(Credential::from_modules(&item, &secrets).is_ok());
    let key_id = item.aws_bindings.as_ref().unwrap().access_key_id.unwrap();
    item.modules
        .iter_mut()
        .find(|module| module.id == key_id)
        .unwrap()
        .name = "renamed access key".into();
    secrets
        .named_secrets
        .insert("renamed access key".into(), "ASIAEXAMPLE".into());
    assert!(Credential::from_modules(&item, &secrets).is_err());
    secrets
        .named_secrets
        .insert("session token".into(), "synthetic-token".into());
    assert!(Credential::from_modules(&item, &secrets).is_ok());
    item.modules
        .retain(|module| Some(module.id) != item.aws_bindings.as_ref().unwrap().secret_access_key);
    assert!(validate_modules(&item.modules, &secrets, item.aws_bindings.as_ref()).is_ok());
    assert!(Credential::from_modules(&item, &secrets).is_err());
}

#[test]
fn invalid_and_duplicate_roles_are_not_credential_fallbacks() {
    let (mut item, secrets) = module_fixture("AKIDEXAMPLE", "");
    item.aws_bindings.as_mut().unwrap().secret_access_key =
        item.aws_bindings.as_ref().unwrap().access_key_id;
    assert!(validate_modules(&item.modules, &secrets, item.aws_bindings.as_ref()).is_err());
    let (mut item, secrets) = module_fixture("AKIDEXAMPLE", "");
    item.aws_bindings.as_mut().unwrap().access_key_id =
        item.aws_bindings.as_ref().unwrap().service_url;
    assert!(validate_modules(&item.modules, &secrets, item.aws_bindings.as_ref()).is_err());
    let (mut item, secrets) = module_fixture("AKIDEXAMPLE", "");
    item.aws_bindings.as_mut().unwrap().region = None;
    assert!(validate_modules(&item.modules, &secrets, item.aws_bindings.as_ref()).is_ok());
    assert!(configured_region(&item, &secrets).is_err());
}
