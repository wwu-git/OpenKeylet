use super::*;
use std::sync::Arc;
use tokio::io::AsyncReadExt;
use tokio_rustls::{
    TlsAcceptor,
    rustls::{ServerConfig, pki_types::PrivateKeyDer},
};

const AWS_HOST: &str = "ec2.us-east-1.amazonaws.com";
const ACCESS: &str = "ASIAMOCKEXAMPLE";
const SECRET: &str = "synthetic-aws-secret-for-provider-test";
const TOKEN: &str = "synthetic-aws-session-token";

fn connection(url: &str, aws: bool) -> (tempfile::TempDir, DecryptedConnection) {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::open(directory.path().join("vault")).unwrap();
    let ids: Vec<_> = (0..5).map(|_| Uuid::new_v4()).collect();
    let value = if aws {
        serde_json::json!({"name":"Synthetic AWS","itemType":"api","httpAuthType":"aws-sigv4","awsBindings":{"serviceUrl":ids[0],"region":ids[1],"accessKeyId":ids[2],"secretAccessKey":ids[3],"sessionToken":ids[4]},"modules":[{"id":ids[0],"kind":"url","value":url},{"id":ids[1],"kind":"customSecret","name":"region"},{"id":ids[2],"kind":"customSecret","name":"access"},{"id":ids[3],"kind":"customSecret","name":"secret"},{"id":ids[4],"kind":"customSecret","name":"session"}],"secrets":{"namedSecrets":{"region":"us-east-1","access":ACCESS,"secret":SECRET,"session":TOKEN}}})
    } else {
        serde_json::json!({"name":"Synthetic token API","itemType":"api","httpAuthType":"bearer","modules":[{"kind":"url","value":url},{"kind":"apiCredential"}],"secrets":{"namedSecrets":{"apiCredential":TOKEN}}})
    };
    let saved = vault
        .save_connection(serde_json::from_value(value).unwrap())
        .unwrap();
    (directory, vault.get_connection(saved.id).unwrap())
}

#[tokio::test]
async fn aws_check_reports_local_readiness_without_claiming_cloud_authentication() {
    let (directory, mut saved) = connection("https://ec2.us-east-1.amazonaws.com/", true);
    let vault = Vault::open(directory.path().join("vault")).unwrap();
    assert_eq!(saved.stored.test_target(&saved.secrets), Some("aws"));
    assert!(saved.stored.public(Some(&saved.secrets)).can_test);
    let summary = test_connection(&vault, &saved).await.unwrap();
    assert!(summary.contains("AWS configuration valid"), "{summary}");
    assert!(summary.contains("not verified"), "{summary}");
    for secret in [ACCESS, SECRET, TOKEN] {
        assert!(!summary.contains(secret));
    }

    saved.stored.http_auth_type = "auto".into();
    saved.stored.aws_bindings = None;
    assert_eq!(saved.stored.test_target(&saved.secrets), Some("aws"));
    let error = test_connection(&vault, &saved).await.unwrap_err().to_string();
    assert!(error.contains("AWS SigV4") && error.contains("AWS signing modules"), "{error}");
    assert!(!error.contains("Bearer fallback"));
}

async fn endpoint(
    host: &str,
    status: &str,
    location: &str,
) -> (
    std::net::SocketAddr,
    reqwest::Certificate,
    tokio::task::JoinHandle<(String, Vec<u8>)>,
) {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec![host.to_owned()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let root = reqwest::Certificate::from_der(cert.der()).unwrap();
    let server = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        PrivateKeyDer::Pkcs8(key.serialize_der().into()),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let status = status.to_owned();
    let location = location.to_owned();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream = TlsAcceptor::from(Arc::new(server))
            .accept(stream)
            .await
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(stream.read_u8().await.unwrap());
            assert!(header.len() < 64 * 1024);
        }
        let header = String::from_utf8(header).unwrap();
        let headers: HashMap<_, _> = header
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':'))
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
            .collect();
        let mut body = vec![
            0;
            headers
                .get("content-length")
                .map_or(0, |v| v.parse().unwrap())
        ];
        stream.read_exact(&mut body).await.unwrap();
        let auth = headers.get("authorization").cloned().unwrap_or_default();
        let signature = auth.rsplit_once("Signature=").map_or("", |(_, v)| v);
        let echoed = serde_json::json!({"authorization":auth,"signature":signature,"key":SECRET,"token":TOKEN}).to_string();
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nx-echo-token: {TOKEN}\r\nx-echo-auth: {auth}\r\nLink: <https://api.github.com/user/repos?page=2>; rel=next\r\n{}\r\n{echoed}",
            echoed.len(),
            if location.is_empty() {
                String::new()
            } else {
                format!("Location: {location}\r\n")
            }
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
        (header, body)
    });
    (address, root, task)
}

async fn run_native(
    connection: &DecryptedConnection,
    input: ApiRequestInput,
    address: std::net::SocketAddr,
    cert: reqwest::Certificate,
) -> Result<ApiResponse> {
    let target = build_api_url(&connection.stored, &input.url, &input.query)?;
    let plan =
        crate::api_provider::execution_plan(&connection.stored, &connection.secrets, &target)?;
    let host = target.host_str().unwrap().to_owned();
    execute_native_request_with(connection, input, target, plan, |timeout| {
        Ok(crate::oci::client_builder(timeout)
            .add_root_certificate(cert)
            .resolve(&host, address)
            .build()?)
    })
    .await
}

#[tokio::test]
async fn aws_final_wire_form_and_json_are_signed_and_echoes_redacted() {
    let (_dir, connection) = connection(&format!("https://{AWS_HOST}/"), true);
    for json_body in [false, true] {
        let (address, cert, server) = endpoint(AWS_HOST, "200 OK", "").await;
        let input = ApiRequestInput {
            url: "/?duplicate=b&duplicate=a&space=two+words&encoded=%2F".into(),
            method: "POST".into(),
            body: json_body.then(|| serde_json::json!({"hello":"雪","value":"a+b"})),
            form: if json_body {
                HashMap::new()
            } else {
                HashMap::from([
                    ("Action".into(), serde_json::json!("DescribeInstances")),
                    ("Version".into(), serde_json::json!("2016-11-15")),
                ])
            },
            headers: HashMap::from([("X-Amz-Target".into(), "Synthetic.Target".into())]),
            ..Default::default()
        };
        let response = run_native(&connection, input, address, cert).await.unwrap();
        let (header, body) = server.await.unwrap();
        assert!(header.contains("AWS4-HMAC-SHA256"));
        assert!(header.contains("/us-east-1/ec2/aws4_request"));
        assert!(
            header
                .to_ascii_lowercase()
                .contains("x-amz-security-token:")
        );
        assert!(header.to_ascii_lowercase().contains("x-amz-target:"));
        let output = serde_json::to_string(&response).unwrap();
        for secret in [SECRET, TOKEN] {
            assert!(!output.contains(secret));
        }
        assert!(!response.body.contains("AWS4-HMAC-SHA256"));
        if json_body {
            assert!(String::from_utf8_lossy(&body).contains('雪'));
        } else {
            assert!(String::from_utf8_lossy(&body).contains("Action=DescribeInstances"));
        }
        if let Ok(directory) = std::env::var("OPENKEYLET_AWS_ORACLE_DIR") {
            let headers: HashMap<_, _> = header
                .lines()
                .skip(1)
                .filter_map(|line| line.split_once(':'))
                .map(|(k, v)| (k.to_ascii_lowercase(), v.trim()))
                .collect();
            let request_target = header
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let evidence = serde_json::json!({"method":"POST","url":format!("https://{AWS_HOST}{request_target}"),"headers":headers,"bodyBase64":STANDARD.encode(body),"accessKeyId":ACCESS,"secretAccessKey":SECRET,"sessionToken":TOKEN,"region":"us-east-1","service":"ec2"});
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(
                PathBuf::from(directory).join(if json_body {
                    "wire-json.json"
                } else {
                    "wire-form.json"
                }),
                evidence.to_string(),
            )
            .unwrap();
        }
    }
}

#[tokio::test]
async fn token_provider_protocol_headers_and_own_host_are_preserved() {
    for url in [
        "https://api.github.com/",
        "https://api.cloudflare.com/client/v4/",
        "https://api.digitalocean.com/v2/",
        "https://api.hetzner.cloud/v1/",
    ] {
        let (_dir, connection) = connection(url, false);
        let host = Url::parse(url).unwrap().host_str().unwrap().to_owned();
        let (address, cert, server) = endpoint(&host, "200 OK", "").await;
        let response = run_native(
            &connection,
            ApiRequestInput {
                method: "GET".into(),
                ..Default::default()
            },
            address,
            cert,
        )
        .await
        .unwrap();
        let (header, _) = server.await.unwrap();
        let lower = header.to_ascii_lowercase();
        assert!(lower.contains(&format!("authorization: bearer {TOKEN}")));
        assert!(lower.contains("user-agent: openkeylet/"));
        if host == "api.github.com" {
            assert!(lower.contains("accept: application/vnd.github+json"));
            assert!(lower.contains("x-github-api-version: 2022-11-28"));
        }
        assert!(!serde_json::to_string(&response).unwrap().contains(TOKEN));
        assert!(response.headers.contains_key("link"));
    }
}

#[tokio::test]
async fn native_redirects_never_reach_b_and_invalid_targets_fail_before_upload() {
    let (_dir, connection) = connection(&format!("https://{AWS_HOST}/"), true);
    let b = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let (address, cert, server) = endpoint(
        AWS_HOST,
        "307 Temporary Redirect",
        &format!("http://{}/leak", b.local_addr().unwrap()),
    )
    .await;
    assert!(
        run_native(
            &connection,
            ApiRequestInput {
                method: "GET".into(),
                ..Default::default()
            },
            address,
            cert
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("redirect blocked")
    );
    server.await.unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), b.accept())
            .await
            .is_err()
    );
    for url in [
        "http://ec2.us-east-1.amazonaws.com/",
        "https://ec2.us-west-2.amazonaws.com/",
        "https://api.github.com/",
        "https://ec2.us-east-1.amazonaws.com.evil.invalid/",
        "https://s3.us-east-1.amazonaws.com/",
    ] {
        let result = execute_api(
            &connection,
            ApiRequestInput {
                url: url.into(),
                method: "POST".into(),
                files: vec![ApiUploadFile {
                    field: "file".into(),
                    path: "/definitely-missing-provider-file".into(),
                    file_name: None,
                    content_type: None,
                }],
                ..Default::default()
            },
        )
        .await;
        let error = result.unwrap_err().to_string();
        assert!(
            !error.contains("uploaded file"),
            "Target must be rejected before file access: {error}"
        );
    }
}

#[tokio::test]
async fn native_response_files_redact_generated_authorization_and_tokens() {
    let (directory, connection) = connection(&format!("https://{AWS_HOST}/"), true);
    let (address, cert, server) = endpoint(AWS_HOST, "200 OK", "").await;
    let path = directory.path().join("response.json");
    let result = run_native(
        &connection,
        ApiRequestInput {
            method: "GET".into(),
            save_response_to: Some(path.to_string_lossy().into()),
            ..Default::default()
        },
        address,
        cert,
    )
    .await
    .unwrap();
    server.await.unwrap();
    assert!(result.saved_to.is_some());
    let text = std::fs::read_to_string(path).unwrap();
    for value in [SECRET, TOKEN, "AWS4-HMAC-SHA256"] {
        assert!(!text.contains(value));
    }
}

#[tokio::test]
async fn environment_proxy_cannot_receive_native_provider_credentials() {
    if std::env::var_os("OPENKEYLET_PROVIDER_PROXY_CHILD").is_some() {
        let (_dir, connection) = connection(&format!("https://{AWS_HOST}/"), true);
        let (address, cert, server) = endpoint(AWS_HOST, "200 OK", "").await;
        run_native(
            &connection,
            ApiRequestInput {
                method: "GET".into(),
                ..Default::default()
            },
            address,
            cert,
        )
        .await
        .unwrap();
        server.await.unwrap();
        return;
    }
    let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("http://{}", proxy.local_addr().unwrap());
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
    child.args(["--exact","executor::provider_transport_tests::environment_proxy_cannot_receive_native_provider_credentials","--nocapture"]).env("OPENKEYLET_PROVIDER_PROXY_CHILD","1");
    for key in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ] {
        child.env(key, &address);
    }
    child.env("NO_PROXY", "").env("no_proxy", "");
    let output = child.output().await.unwrap();
    assert!(
        output.status.success(),
        "Isolated proxy regression failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(100), proxy.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stored_token_auth_cannot_inject_native_transport_headers() {
    let (_dir, mut connection) = connection("https://api.cloudflare.com/client/v4/", false);
    for mode in ["custom", "apiKey"] {
        for name in [
            "Host",
            "content-length",
            "transfer-encoding",
            "proxy-authorization",
        ] {
            connection.stored.http_auth_type = mode.into();
            connection.stored.auth_header = name.into();
            connection.stored.auth_location = "header".into();
            connection.stored.api_auth_headers = vec![crate::model::ApiAuthHeader {
                name: name.into(),
                secret_name: "apiCredential".into(),
            }];
            let input = ApiRequestInput {
                method: "GET".into(),
                ..Default::default()
            };
            let target = build_api_url(&connection.stored, "", &HashMap::new()).unwrap();
            let plan = crate::api_provider::execution_plan(
                &connection.stored,
                &connection.secrets,
                &target,
            )
            .unwrap();
            let error = execute_native_request_with(&connection, input, target, plan, |timeout| {
                Ok(crate::oci::client_builder(timeout).build()?)
            })
            .await
            .unwrap_err()
            .to_string();
            assert!(error.contains("transport header"));
            assert!(!error.contains(TOKEN));
        }
    }
}
