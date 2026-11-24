use super::*;
use crate::oci::{
    self,
    tests::{HOST, KEY, config},
};
use std::sync::Arc;
const URI_SECRET: &str = "hidden space/雪?query secret/終";
const URI_ACTUAL: &str = "hidden%20space/%E9%9B%AA?query%20secret/%E7%B5%82";
const URI_PATH: &str = "hidden%20space/%E9%9B%AA%3Fquery%20secret/%E7%B5%82";
const URI_QUERY: &str = "hidden%20space%2F%E9%9B%AA%3Fquery%20secret%2F%E7%B5%82";
const URI_FORM: &str = "hidden+space%2F%E9%9B%AA%3Fquery+secret%2F%E7%B5%82";
use tokio::io::AsyncReadExt;
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
};

#[tokio::test]
async fn generic_oci_requests_sign_exact_wire_bytes_and_redact_all_responses() {
    let cert = include_bytes!("../tests/fixtures/oci/generic-endpoint.pem");
    let tls = ServerConfig::builder_with_provider(Arc::new(
        tokio_rustls::rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from_pem_slice(cert).unwrap()],
        PrivateKeyDer::from_pem_slice(KEY.as_bytes()).unwrap(),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let upload = directory.path().join("upload.bin");
    tokio::fs::write(&upload, [0xff, 0x00, b'x', b'\n'])
        .await
        .unwrap();
    let output = directory.path().join("download.bin");
    let existing = directory.path().join("unchanged.bin");
    tokio::fs::write(&existing, b"old-data").await.unwrap();
    let base = |method: &str, path: &str| ApiRequestInput {
        url: format!("https://{HOST}{path}"),
        method: method.into(),
        timeout_seconds: Some(5),
        ..Default::default()
    };
    let inputs = vec![
        ApiRequestInput { url: "https://identity.ap-tokyo-1.oci.oraclecloud.com/20160918/compartments?literal=+&encoded=%2F&repeat=1&repeat=2".into(), query: HashMap::from([("compartmentIdInSubtree".into(), Value::Bool(true)), ("newParameter".into(), serde_json::json!(["one", "two"]))]), ..base("GET", "/") },
        ApiRequestInput { url: "https://new-service.ap-tokyo-1.oraclecloud.com/new-api/objects".into(), ..base("HEAD", "/") },
        base("DELETE", "/new-api/items/id"),
        base("OPTIONS", "/new-api/options"),
        ApiRequestInput { body: Some(serde_json::json!({"name":"雪", "flag":true})), ..base("POST", "/new-api/items") },
        base("PUT", "/new-api/empty"),
        ApiRequestInput { body: Some(Value::String("patch 雪".into())), headers: HashMap::from([("Content-Type".into(), "text/plain; charset=utf-8".into())]), ..base("PATCH", "/new-api/patch") },
        ApiRequestInput { form: HashMap::from([("values".into(), serde_json::json!(["one two", "雪"])), ("flag".into(), Value::Bool(true))]), ..base("POST", "/new-api/form") },
        ApiRequestInput { body_base64: Some(STANDARD.encode([0xff, 0x00, 0xfe])), ..base("PUT", "/new-api/binary") },
        ApiRequestInput { files: vec![ApiUploadFile { field:"file".into(), path:upload.to_string_lossy().into_owned(), file_name:Some("test.bin".into()), content_type:Some("application/octet-stream".into()) }], form:HashMap::from([("name".into(), Value::String("雪".into()))]), headers:HashMap::from([("Content-Type".into(), "application/json".into())]), ..base("POST", "/new-api/multipart") },
        base("GET", &format!("/uri-echo/{URI_SECRET}")),
        base("GET", "/error"),
        ApiRequestInput { save_response_to:Some(output.to_string_lossy().into_owned()), ..base("GET", "/binary-response") },
        ApiRequestInput { save_response_to:Some(existing.to_string_lossy().into_owned()), overwrite_response_file:true, max_response_bytes:Some(4), ..base("GET", "/large-response") },
    ];
    let request_count = inputs.len();
    let server = tokio::spawn(async move {
        let acceptor = TlsAcceptor::from(Arc::new(tls));
        let mut requests = Vec::new();
        for _ in 0..request_count {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = acceptor.accept(stream).await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(stream.read_u8().await.unwrap());
                assert!(header.len() < 32 * 1024);
            }
            let header = String::from_utf8(header).unwrap();
            let headers = header
                .lines()
                .skip(1)
                .filter_map(|line| line.split_once(": "))
                .map(|(key, value)| (key.to_ascii_lowercase(), value.to_owned()))
                .collect::<HashMap<_, _>>();
            let length = headers
                .get("content-length")
                .map_or(0, |value| value.parse::<usize>().unwrap());
            let mut body = vec![0; length];
            stream.read_exact(&mut body).await.unwrap();
            let target = header
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let auth = &headers["authorization"];
            let (status, response_body) = if target == "/error" {
                ("400 Bad Request", serde_json::to_string(&serde_json::json!({"code":"InvalidParameter", "reason":"ordinary explanation", "echo":auth, "secret":"test-secret", "quoted":"quoted\n\"雪"})).unwrap().replace('/', "\\/").into_bytes())
            } else if target.starts_with("/uri-echo/") {
                (
                    "200 OK",
                    serde_json::to_string(
                        &serde_json::json!({"values":[URI_ACTUAL,URI_PATH,URI_QUERY,URI_FORM]}),
                    )
                    .unwrap()
                    .replace('/', "\\/")
                    .into_bytes(),
                )
            } else if target == "/binary-response" {
                let mut bytes = vec![0xff, 0x00];
                bytes.extend_from_slice(auth.as_bytes());
                bytes.extend_from_slice(b"test-secret");
                for encoded in [URI_ACTUAL, URI_PATH, URI_QUERY, URI_FORM] {
                    bytes.push(b'|');
                    bytes.extend_from_slice(encoded.as_bytes());
                }
                bytes.push(0xfe);
                ("200 OK", bytes)
            } else if target == "/large-response" {
                ("200 OK", b"too-large".to_vec())
            } else if header.starts_with("HEAD ") {
                ("200 OK", Vec::new())
            } else {
                ("200 OK", b"[]".to_vec())
            };
            let response_header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\nopc-next-page: a+b c\r\nopc-work-request-id: work-fixture\r\netag: etag-fixture\r\nAuthorization: {auth}\r\nSet-Cookie: test-secret\r\nx-echo-signature: {auth}\r\nx-echo-secret: test-secret\r\nLocation: https://{HOST}/objects/{URI_ACTUAL}\r\nx-path-echo: {URI_PATH}\r\nx-rfc3986-echo: {URI_QUERY}\r\nx-form-echo: {URI_FORM}\r\n\r\n",
                response_body.len()
            );
            stream.write_all(response_header.as_bytes()).await.unwrap();
            stream.write_all(&response_body).await.unwrap();
            stream.shutdown().await.unwrap();
            requests.push((header, body));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), listener.accept())
                .await
                .is_err()
        );
        requests
    });
    let vault = Vault::open(directory.path().join("vault")).unwrap();
    let saved = vault
        .save_oci_connection(
            serde_json::from_value(serde_json::json!({"name":"generic OCI test"})).unwrap(),
            None,
            config(),
            Some(KEY),
            None,
        )
        .unwrap();
    let mut connection = vault.get_connection(saved.id).unwrap();
    connection.secrets.token = Some("test-secret".into());
    connection
        .secrets
        .named_secrets
        .0
        .insert("quoted".into(), "quoted\n\"雪".into());
    connection
        .secrets
        .named_secrets
        .0
        .insert("uri".into(), URI_SECRET.into());
    let client = |timeout| {
        Ok(oci::client_builder(timeout)
            .add_root_certificate(reqwest::Certificate::from_pem(cert).unwrap())
            .resolve(HOST, addr)
            .resolve("identity.ap-tokyo-1.oci.oraclecloud.com", addr)
            .resolve("new-service.ap-tokyo-1.oraclecloud.com", addr)
            .build()?)
    };
    for (index, input) in inputs.into_iter().enumerate() {
        let result = execute_oci_request_with(&connection, input, client).await;
        if index == request_count - 1 {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("no response file was saved")
            );
            assert_eq!(tokio::fs::read(&existing).await.unwrap(), b"old-data");
            continue;
        }
        let result = result.unwrap();
        assert_eq!(result.headers["opc-next-page"], "a+b c");
        assert_eq!(result.headers["opc-work-request-id"], "work-fixture");
        assert_eq!(result.headers["etag"], "etag-fixture");
        assert_eq!(result.headers["x-echo-signature"], "[REDACTED]");
        assert_eq!(result.headers["x-echo-secret"], "[REDACTED]");
        assert_eq!(
            result.headers["location"],
            format!("https://{}/objects/[REDACTED]", HOST.replace("ap-tokyo-1", "[REDACTED]"))
        );
        for header in ["x-path-echo", "x-rfc3986-echo", "x-form-echo"] {
            assert_eq!(result.headers[header], "[REDACTED]");
        }
        if index == request_count - 4 {
            assert_eq!(result.url, format!("https://{}/uri-echo/[REDACTED]", HOST.replace("ap-tokyo-1", "[REDACTED]")));
            let body: Value = serde_json::from_str(&result.body).unwrap();
            assert_eq!(
                body["values"],
                serde_json::json!(["[REDACTED]", "[REDACTED]", "[REDACTED]", "[REDACTED]"])
            );
        }
        assert!(!result.headers.contains_key("authorization"));
        assert!(!result.headers.contains_key("set-cookie"));
        if index == request_count - 3 {
            assert_eq!(result.status, 400);
            let error: Value = serde_json::from_str(&result.body).unwrap();
            assert_eq!(error["code"], "InvalidParameter");
            assert_eq!(error["reason"], "ordinary explanation");
            assert_eq!(error["echo"], "[REDACTED]");
            assert_eq!(error["secret"], "[REDACTED]");
            assert_eq!(error["quoted"], "[REDACTED]");
        } else if index == request_count - 2 {
            assert_eq!(
                result.saved_to.as_deref(),
                Some(std::fs::canonicalize(&output).unwrap().to_str().unwrap())
            );
            assert!(result.body.is_empty());
            let expected =
                b"\xff\0[REDACTED][REDACTED]|[REDACTED]|[REDACTED]|[REDACTED]|[REDACTED]\xfe";
            assert_eq!(tokio::fs::read(&output).await.unwrap(), expected);
            assert_eq!(result.bytes_transferred, Some(expected.len() as u64));
        } else {
            assert_eq!(result.status, 200);
        }
    }
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), request_count);
    let pem_body: String = KEY
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let key = ring::signature::RsaKeyPair::from_pkcs8(&STANDARD.decode(pem_body).unwrap()).unwrap();
    let verifier = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        key.public().as_ref(),
    );
    for (header, body) in &requests {
        let request_line = header
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .collect::<Vec<_>>();
        let headers = header
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(": "))
            .map(|(key, value)| (key.to_ascii_lowercase(), value))
            .collect::<HashMap<_, _>>();
        let authorization = headers["authorization"];
        let names = authorization
            .split("headers=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        for required in ["(request-target)", "host", "x-date"] {
            assert!(names.split_whitespace().any(|name| name == required));
        }
        let message = names
            .split_whitespace()
            .map(|name| {
                if name == "(request-target)" {
                    format!(
                        "{name}: {} {}",
                        request_line[0].to_ascii_lowercase(),
                        request_line[1]
                    )
                } else {
                    format!("{name}: {}", headers[name])
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let signature = authorization
            .rsplit_once("signature=\"")
            .unwrap()
            .1
            .trim_end_matches('"');
        verifier
            .verify(message.as_bytes(), &STANDARD.decode(signature).unwrap())
            .unwrap();
        if matches!(request_line[0], "POST" | "PUT" | "PATCH") || !body.is_empty() {
            use sha2::Digest;
            assert_eq!(
                headers["x-content-sha256"],
                STANDARD.encode(sha2::Sha256::digest(body))
            );
            assert_eq!(
                headers["content-length"].parse::<usize>().unwrap(),
                body.len()
            );
            for required in ["x-content-sha256", "content-type", "content-length"] {
                assert!(names.split_whitespace().any(|name| name == required));
            }
            assert!(
                header
                    .lines()
                    .filter(|line| line.to_ascii_lowercase().starts_with("content-type:"))
                    .count()
                    == 1
            );
        }
    }
    assert!(
        requests[0]
            .0
            .contains("literal=+&encoded=%2F&repeat=1&repeat=2")
    );
    assert!(requests[0].0.contains("compartmentIdInSubtree=true"));
    assert_eq!(
        requests[4].1,
        serde_json::to_vec(&serde_json::json!({"name":"雪","flag":true})).unwrap()
    );
    assert_eq!(requests[5].1, b"");
    assert_eq!(requests[8].1, [0xff, 0x00, 0xfe]);
    let multipart = &requests[9];
    let content_type = multipart
        .0
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("content-type:"))
        .unwrap();
    assert!(content_type.contains("multipart/form-data; boundary="));
    let boundary = content_type.split("boundary=").nth(1).unwrap();
    assert!(
        multipart
            .1
            .starts_with(format!("--{boundary}\r\n").as_bytes())
    );
    assert!(
        multipart
            .1
            .windows(4)
            .any(|bytes| bytes == [0xff, 0x00, b'x', b'\n'])
    );
    assert!(
        multipart
            .1
            .windows("雪".len())
            .any(|bytes| bytes == "雪".as_bytes())
    );

    for input in [
        ApiRequestInput {
            body: Some(Value::String("body".into())),
            body_base64: Some("Ym9keQ==".into()),
            ..base("POST", "/conflict")
        },
        ApiRequestInput {
            body: Some(Value::String("x".repeat(MAX_OCI_REQUEST_BYTES + 1))),
            ..base("POST", "/oversized")
        },
    ] {
        let error = execute_oci_request_with(&connection, input, client)
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("body format") || error.contains("64 MiB"));
    }
    let mut invalid = base("POST", "/invalid");
    invalid.url = "https://outside.invalid/request".into();
    invalid.files = vec![ApiUploadFile {
        field: "file".into(),
        path: "/does-not-exist".into(),
        file_name: None,
        content_type: None,
    }];
    let error = execute_oci_request_with(&connection, invalid, |_| {
        panic!("invalid target reached client")
    })
    .await
    .unwrap_err();
    assert!(error.to_string().contains("outside"));
    connection.stored.enabled = false;
    assert!(
        execute_oci_request_with(&connection, base("GET", "/"), |_| panic!(
            "disabled item reached client"
        ))
        .await
        .unwrap_err()
        .to_string()
        .contains("disabled")
    );
}
