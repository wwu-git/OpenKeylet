use crate::{
    api_provider::ProviderId,
    executor::{ApiRequestInput, ApiResponse},
    vault::DecryptedConnection,
};
use quick_xml::{Reader, events::Event};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(
    tag = "operation",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum NativeWorkflow {
    GithubCommits {
        owner: Option<String>,
        repo: Option<String>,
        page: Option<u32>,
    },
    OciRunningInstances {
        compartment_name: Option<String>,
        compartment_id: Option<String>,
        page: Option<String>,
    },
    AwsRunningInstances {
        next_token: Option<String>,
    },
    CloudflareDnsRecords {
        zone_name: Option<String>,
        zone_id: Option<String>,
        page: Option<u32>,
    },
    DigitalOceanDroplets {
        page: Option<u32>,
    },
    HetznerServers {
        page: Option<u32>,
    },
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeLookup {
    match_name: String,
    collection_pointer: &'static str,
    match_field: &'static str,
    id_field: &'static str,
    parent_pointer: &'static str,
    next_input: &'static str,
    collect_all_pages: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    matches: Option<Vec<NativeMatch>>,
}

#[derive(Serialize, JsonSchema)]
struct NativeMatch {
    id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
}

impl NativeLookup {
    fn new(
        name: &str,
        collection_pointer: &'static str,
        parent_pointer: &'static str,
        next_input: &'static str,
    ) -> Self {
        Self {
            match_name: name.to_owned(),
            collection_pointer,
            match_field: "name",
            id_field: "id",
            parent_pointer,
            next_input,
            collect_all_pages: true,
            matches: None,
        }
    }

    pub(crate) fn include_matches(&mut self, data: &Value) {
        self.matches = data
            .pointer(self.collection_pointer)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        let name = item.get(self.match_field)?.as_str()?;
                        if !name.eq_ignore_ascii_case(&self.match_name) {
                            return None;
                        }
                        let id = item.get(self.id_field)?.as_str()?;
                        if id.contains("[REDACTED]") {
                            return None;
                        }
                        Some(NativeMatch {
                            id: id.to_owned(),
                            name: name.to_owned(),
                            parent: item
                                .pointer(self.parent_pointer)
                                .and_then(Value::as_str)
                                .map(str::to_owned),
                        })
                    })
                    .collect()
            });
    }
}

pub(crate) struct PreparedNativeRequest {
    pub(crate) request: ApiRequestInput,
    pub(crate) lookup: Option<NativeLookup>,
}

#[derive(Debug)]
pub(crate) struct NativeInputIssue {
    pub(crate) field: &'static str,
    pub(crate) source: &'static str,
    pub(crate) invalid: bool,
}

impl NativeInputIssue {
    fn caller(field: &'static str, invalid: bool) -> Self {
        Self {
            field,
            source: "caller",
            invalid,
        }
    }

    fn project(field: &'static str) -> Self {
        Self {
            field,
            source: "project",
            invalid: false,
        }
    }
}

impl NativeWorkflow {
    pub(crate) fn provider(&self) -> ProviderId {
        match self {
            Self::GithubCommits { .. } => ProviderId::Github,
            Self::OciRunningInstances { .. } => ProviderId::Oci,
            Self::AwsRunningInstances { .. } => ProviderId::Aws,
            Self::CloudflareDnsRecords { .. } => ProviderId::Cloudflare,
            Self::DigitalOceanDroplets { .. } => ProviderId::DigitalOcean,
            Self::HetznerServers { .. } => ProviderId::Hetzner,
        }
    }

    pub(crate) fn prepare(
        &self,
        connection: &DecryptedConnection,
    ) -> Result<PreparedNativeRequest, NativeInputIssue> {
        let mut request = ApiRequestInput {
            method: "GET".to_owned(),
            ..Default::default()
        };
        let mut lookup = None;
        request.url = match self {
            Self::GithubCommits { owner, repo, page } => {
                let owner = required(owner, "owner", |value| path_segment(value, 100))?;
                let repo = required(repo, "repo", |value| path_segment(value, 100))?;
                number_page(&mut request.query, *page)?;
                format!("https://api.github.com/repos/{owner}/{repo}/commits")
            }
            Self::OciRunningInstances {
                compartment_name,
                compartment_id,
                page,
            } => {
                let bindings = connection
                    .stored
                    .oci_bindings
                    .as_ref()
                    .ok_or_else(|| NativeInputIssue::project("ociBindings"))?;
                let bound = |id| {
                    crate::oci::binding_value(&connection.stored.modules, &connection.secrets, id)
                };
                let region = bound(bindings.region)
                    .filter(|region| crate::oci::REGIONS.contains(region))
                    .ok_or_else(|| NativeInputIssue::project("region"))?;
                if let Some(page) = checked_cursor(page, "page")? {
                    request.query.insert("page".to_owned(), json!(page));
                }
                request.query.insert("limit".to_owned(), json!(50));
                if compartment_id.is_some() {
                    let id = required(compartment_id, "compartmentId", valid_oci_compartment)?;
                    request.query.insert("compartmentId".to_owned(), json!(id));
                    request
                        .query
                        .insert("lifecycleState".to_owned(), json!("RUNNING"));
                    format!(
                        "https://iaas.{region}.oraclecloud.com{}",
                        crate::oci::INSTANCES_PATH
                    )
                } else {
                    let name = required(compartment_name, "compartmentName", |value| {
                        resource_name(value, 100)
                    })?;
                    let tenancy = bound(bindings.tenancy_ocid)
                        .filter(|id| valid_ocid(id, "ocid1.tenancy.oc1.."))
                        .ok_or_else(|| NativeInputIssue::project("tenancyOcid"))?;
                    request.query.extend([
                        ("compartmentId".to_owned(), json!(tenancy)),
                        ("compartmentIdInSubtree".to_owned(), json!(true)),
                        ("accessLevel".to_owned(), json!("ACCESSIBLE")),
                        ("lifecycleState".to_owned(), json!("ACTIVE")),
                    ]);
                    lookup = Some(NativeLookup::new(
                        name,
                        "",
                        "/compartmentId",
                        "compartmentId",
                    ));
                    format!("https://identity.{region}.oci.oraclecloud.com/20160918/compartments")
                }
            }
            Self::AwsRunningInstances { next_token } => {
                let region = crate::aws::configured_region(&connection.stored, &connection.secrets)
                    .map_err(|_| NativeInputIssue::project("region"))?;
                request.query.extend([
                    ("Action".to_owned(), json!("DescribeInstances")),
                    ("Version".to_owned(), json!("2016-11-15")),
                    ("Filter.1.Name".to_owned(), json!("instance-state-name")),
                    ("Filter.1.Value.1".to_owned(), json!("running")),
                    ("MaxResults".to_owned(), json!(50)),
                ]);
                if let Some(token) = checked_cursor(next_token, "nextToken")? {
                    request.query.insert("NextToken".to_owned(), json!(token));
                }
                format!("https://ec2.{region}.amazonaws.com/")
            }
            Self::CloudflareDnsRecords {
                zone_name,
                zone_id,
                page,
            } => {
                number_page(&mut request.query, *page)?;
                if zone_id.is_some() {
                    let id = required(zone_id, "zoneId", |value| {
                        value.len() == 32 && value.bytes().all(|c| c.is_ascii_hexdigit())
                    })?;
                    format!("https://api.cloudflare.com/client/v4/zones/{id}/dns_records")
                } else {
                    let name = required(zone_name, "zoneName", |value| path_segment(value, 253))?;
                    request.query.insert("name".to_owned(), json!(name));
                    lookup = Some(NativeLookup::new(name, "/result", "/account/id", "zoneId"));
                    "https://api.cloudflare.com/client/v4/zones".to_owned()
                }
            }
            Self::DigitalOceanDroplets { page } => {
                number_page(&mut request.query, *page)?;
                "https://api.digitalocean.com/v2/droplets".to_owned()
            }
            Self::HetznerServers { page } => {
                number_page(&mut request.query, *page)?;
                "https://api.hetzner.cloud/v1/servers".to_owned()
            }
        };
        Ok(PreparedNativeRequest { request, lookup })
    }

    pub(crate) fn next_page(&self, reply: &ApiResponse) -> Option<Self> {
        if !(200..300).contains(&reply.status) || reply.truncated || reply.saved_to.is_some() {
            return None;
        }
        let mut next = self.clone();
        let data = || serde_json::from_str::<Value>(&reply.body).ok();
        match &mut next {
            Self::GithubCommits { page, .. } => {
                let link = github_next_link(reply.headers.get("link")?)?;
                *page = Some(forward_page(*page, linked_page(link, "api.github.com")?)?);
            }
            Self::OciRunningInstances { page, .. } => {
                *page = Some(forward_cursor(page, reply.headers.get("opc-next-page")?)?);
            }
            Self::AwsRunningInstances { next_token } => {
                *next_token = Some(forward_cursor(next_token, &ec2_next_token(&reply.body)?)?);
            }
            Self::CloudflareDnsRecords { page, .. } => {
                let data = data()?;
                if !data.get("success")?.as_bool()? {
                    return None;
                }
                let got = number_field(data.pointer("/result_info/page")?)?;
                let total = number_field(data.pointer("/result_info/total_pages")?)?;
                if got != page.unwrap_or(1) || got >= total {
                    return None;
                }
                *page = Some(got.checked_add(1)?);
            }
            Self::DigitalOceanDroplets { page } => {
                let data = data()?;
                let link = data.pointer("/links/pages/next")?.as_str()?;
                *page = Some(forward_page(
                    *page,
                    linked_page(link, "api.digitalocean.com")?,
                )?);
            }
            Self::HetznerServers { page } => {
                let data = data()?;
                let got = number_field(data.pointer("/meta/pagination/next_page")?)?;
                *page = Some(forward_page(*page, got)?);
            }
        }
        Some(next)
    }
}

fn required<'a>(
    value: &'a Option<String>,
    field: &'static str,
    valid: impl FnOnce(&str) -> bool,
) -> Result<&'a str, NativeInputIssue> {
    let value = value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| NativeInputIssue::caller(field, false))?;
    if !valid(value) {
        return Err(NativeInputIssue::caller(field, true));
    }
    Ok(value)
}

fn path_segment(value: &str, max: usize) -> bool {
    !matches!(value, "." | "..") && resource_name(value, max)
}

fn resource_name(value: &str, max: usize) -> bool {
    value.len() <= max
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
}

fn valid_ocid(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.len() <= 240
            && suffix.bytes().all(|c| c.is_ascii_alphanumeric())
    })
}

fn valid_oci_compartment(value: &str) -> bool {
    valid_ocid(value, "ocid1.compartment.oc1..") || valid_ocid(value, "ocid1.tenancy.oc1..")
}

fn number_page(
    query: &mut HashMap<String, Value>,
    page: Option<u32>,
) -> Result<(), NativeInputIssue> {
    if page == Some(0) {
        return Err(NativeInputIssue::caller("page", true));
    }
    query.insert("per_page".to_owned(), json!(30));
    if let Some(page) = page {
        query.insert("page".to_owned(), json!(page));
    }
    Ok(())
}

fn valid_cursor(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 16 * 1024
        && !value.chars().any(char::is_control)
        && !value.contains("[REDACTED]")
}

fn checked_cursor<'a>(
    value: &'a Option<String>,
    field: &'static str,
) -> Result<Option<&'a str>, NativeInputIssue> {
    match value.as_deref() {
        Some(value) if !valid_cursor(value) => Err(NativeInputIssue::caller(field, true)),
        value => Ok(value),
    }
}

fn forward_cursor(current: &Option<String>, next: &str) -> Option<String> {
    (valid_cursor(next) && current.as_deref() != Some(next)).then(|| next.to_owned())
}

fn forward_page(current: Option<u32>, next: u32) -> Option<u32> {
    (next > current.unwrap_or(1)).then_some(next)
}

fn number_field(value: &Value) -> Option<u32> {
    value.as_u64()?.try_into().ok()
}

fn linked_page(link: &str, host: &str) -> Option<u32> {
    let url = Url::parse(link).ok()?;
    if url.scheme() != "https"
        || url.host_str()? != host
        || url.port_or_known_default()? != 443
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let mut pages = url.query_pairs().filter(|(key, _)| key == "page");
    let page = pages.next()?.1.parse().ok()?;
    pages.next().is_none().then_some(page)
}

fn github_next_link(value: &str) -> Option<&str> {
    value.split(',').find_map(|part| {
        let (url, parameters) = part.trim().strip_prefix('<')?.split_once('>')?;
        parameters
            .split(';')
            .any(|parameter| {
                parameter
                    .trim()
                    .split_once('=')
                    .is_some_and(|(key, value)| {
                        key.trim() == "rel"
                            && value
                                .trim()
                                .trim_matches('"')
                                .split_whitespace()
                                .any(|rel| rel == "next")
                    })
            })
            .then_some(url)
    })
}

fn ec2_next_token(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    let mut depth = 0usize;
    let mut root_seen = false;
    let mut token = None;
    loop {
        match reader.read_event().ok()? {
            Event::Start(tag) if depth == 0 => {
                if root_seen || tag.local_name().as_ref() != b"DescribeInstancesResponse" {
                    return None;
                }
                root_seen = true;
                depth += 1;
            }
            Event::Start(tag) if depth == 1 && tag.local_name().as_ref() == b"nextToken" => {
                if token.is_some() {
                    return None;
                }
                let text = reader.read_text(tag.name()).ok()?;
                let raw = text.decode().ok()?;
                if raw.contains('<') {
                    return None;
                }
                token = Some(quick_xml::escape::unescape(raw.trim()).ok()?.into_owned());
            }
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.checked_sub(1)?,
            Event::DocType(_) => return None,
            Event::Eof => return (depth == 0 && root_seen).then_some(token).flatten(),
            Event::Text(raw) if depth == 0 && !raw.decode().ok()?.trim().is_empty() => return None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AwsModuleBindings, ItemModule, OciModuleBindings};
    use uuid::Uuid;

    fn fixture_project() -> DecryptedConnection {
        let mut project = DecryptedConnection {
            stored: serde_json::from_value(json!({
                "id": Uuid::new_v4(), "name": "fixture", "enabled": true,
                "createdAt": "", "updatedAt": "",
                "encryptedSecrets": {"version": 1, "nonce": "", "ciphertext": ""},
            }))
            .unwrap(),
            secrets: Default::default(),
        };
        for (name, value) in [
            ("AWS region", "us-east-2"),
            ("OCI region", "us-ashburn-1"),
            ("OCI tenancy", "ocid1.tenancy.oc1..fixture"),
        ] {
            project
                .secrets
                .named_secrets
                .insert(name.into(), value.into());
            project.stored.modules.push(ItemModule {
                id: Uuid::new_v4(),
                kind: "customSecret".into(),
                name: name.into(),
                agent_visible: Some(false),
                ..Default::default()
            });
        }
        let modules = &project.stored.modules;
        project.stored.aws_bindings = Some(AwsModuleBindings {
            region: Some(modules[0].id),
            ..Default::default()
        });
        project.stored.oci_bindings = Some(OciModuleBindings {
            region: Some(modules[1].id),
            tenancy_ocid: Some(modules[2].id),
            ..Default::default()
        });
        project
    }

    fn workflow(value: Value) -> NativeWorkflow {
        serde_json::from_value(value).unwrap()
    }

    fn reply(body: &str, header: Option<(&str, &str)>) -> ApiResponse {
        ApiResponse {
            status: 200,
            status_text: String::new(),
            url: String::new(),
            headers: header
                .into_iter()
                .map(|(key, value)| (key.into(), value.into()))
                .collect(),
            body: body.into(),
            truncated: false,
            saved_to: None,
            bytes_transferred: None,
        }
    }

    #[test]
    fn operations_derive_fixed_urls_and_request_parameters() {
        let project = fixture_project();
        let cases = [
            (
                json!({"operation":"githubCommits","owner":"org","repo":"repo"}),
                "https://api.github.com/repos/org/repo/commits",
            ),
            (
                json!({"operation":"ociRunningInstances","compartmentName":"app"}),
                "https://identity.us-ashburn-1.oci.oraclecloud.com/20160918/compartments",
            ),
            (
                json!({"operation":"ociRunningInstances","compartmentId":"ocid1.compartment.oc1..app"}),
                "https://iaas.us-ashburn-1.oraclecloud.com/20160918/instances",
            ),
            (
                json!({"operation":"awsRunningInstances"}),
                "https://ec2.us-east-2.amazonaws.com/",
            ),
            (
                json!({"operation":"cloudflareDnsRecords","zoneName":"example.com"}),
                "https://api.cloudflare.com/client/v4/zones",
            ),
            (
                json!({"operation":"cloudflareDnsRecords","zoneId":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}),
                "https://api.cloudflare.com/client/v4/zones/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/dns_records",
            ),
            (
                json!({"operation":"digitalOceanDroplets"}),
                "https://api.digitalocean.com/v2/droplets",
            ),
            (
                json!({"operation":"hetznerServers"}),
                "https://api.hetzner.cloud/v1/servers",
            ),
        ];
        for (input, url) in cases {
            let operation = workflow(input);
            let prepared = operation.prepare(&project).unwrap();
            assert_eq!(prepared.request.url, url);
            assert_eq!(prepared.request.method, "GET");
            let call = serde_json::to_string(&operation).unwrap();
            assert!(
                !call.contains("ocid1.tenancy")
                    && !call.contains("us-ashburn-1")
                    && !call.contains("us-east-2")
            );
        }
        let lookup = workflow(json!({"operation":"ociRunningInstances","compartmentName":"app"}))
            .prepare(&project)
            .unwrap();
        assert_eq!(
            lookup.request.query["compartmentId"],
            "ocid1.tenancy.oc1..fixture"
        );
        let lookup = serde_json::to_value(lookup.lookup).unwrap();
        assert_eq!(lookup["nextInput"], "compartmentId");
        assert_eq!(lookup["matchName"], "app");
        assert_eq!(lookup["collectAllPages"], true);

        let compute = workflow(json!({"operation":"ociRunningInstances","compartmentId":"ocid1.compartment.oc1..app","page":"opaque page"}))
            .prepare(&project).unwrap();
        assert_eq!(compute.request.query["lifecycleState"], "RUNNING");
        assert_eq!(compute.request.query["page"], "opaque page");
        let aws = workflow(json!({"operation":"awsRunningInstances","nextToken":"token"}))
            .prepare(&project)
            .unwrap();
        assert_eq!(aws.request.query["Action"], "DescribeInstances");
        assert_eq!(aws.request.query["Filter.1.Value.1"], "running");
        assert_eq!(aws.request.query["NextToken"], "token");
    }

    #[test]
    fn missing_or_invalid_inputs_never_make_requests() {
        let project = fixture_project();
        let cases = [
            (
                json!({"operation":"githubCommits","owner":"org"}),
                "repo",
                false,
            ),
            (
                json!({"operation":"githubCommits","owner":"org","repo":"a/b"}),
                "repo",
                true,
            ),
            (
                json!({"operation":"githubCommits","owner":"org","repo":".."}),
                "repo",
                true,
            ),
            (
                json!({"operation":"ociRunningInstances","compartmentId":"app"}),
                "compartmentId",
                true,
            ),
            (
                json!({"operation":"cloudflareDnsRecords","zoneId":"../zones"}),
                "zoneId",
                true,
            ),
            (json!({"operation":"hetznerServers","page":0}), "page", true),
        ];
        for (input, field, invalid) in cases {
            let error = workflow(input).prepare(&project).err().unwrap();
            assert_eq!(
                (error.field, error.source, error.invalid),
                (field, "caller", invalid)
            );
        }
    }

    #[test]
    fn six_native_paginators_keep_filters_and_handle_empty_pages() {
        let cases = [
            (
                json!({"operation":"githubCommits","owner":"org","repo":"repo"}),
                reply(
                    "[]",
                    Some((
                        "link",
                        "<https://api.github.com/repositories/123/commits?page=2>; rel=\"next\"",
                    )),
                ),
                "page",
                json!(2),
            ),
            (
                json!({"operation":"ociRunningInstances","compartmentName":"app"}),
                reply("[]", Some(("opc-next-page", "opaque + page"))),
                "page",
                json!("opaque + page"),
            ),
            (
                json!({"operation":"awsRunningInstances"}),
                reply(
                    "<DescribeInstancesResponse><reservationSet/><nextToken>A&amp;B</nextToken></DescribeInstancesResponse>",
                    None,
                ),
                "nextToken",
                json!("A&B"),
            ),
            (
                json!({"operation":"cloudflareDnsRecords","zoneName":"example.com"}),
                reply(
                    r#"{"success":true,"result":[],"result_info":{"page":1,"total_pages":2}}"#,
                    None,
                ),
                "page",
                json!(2),
            ),
            (
                json!({"operation":"digitalOceanDroplets"}),
                reply(
                    r#"{"droplets":[],"links":{"pages":{"next":"https://api.digitalocean.com/v2/droplets?page=2"}}}"#,
                    None,
                ),
                "page",
                json!(2),
            ),
            (
                json!({"operation":"hetznerServers"}),
                reply(
                    r#"{"servers":[],"meta":{"pagination":{"next_page":2}}}"#,
                    None,
                ),
                "page",
                json!(2),
            ),
        ];
        for (input, response, key, value) in cases {
            let next = workflow(input.clone()).next_page(&response).unwrap();
            let mut expected = input;
            expected[key] = value;
            assert_eq!(next, workflow(expected));
            assert!(next.next_page(&response).is_none());
        }
    }

    #[test]
    fn pagination_refuses_foreign_links_and_failed_or_incomplete_responses() {
        let github = workflow(json!({"operation":"githubCommits"}));
        let foreign = reply(
            "[]",
            Some((
                "link",
                "<https://api.github.com.example.com/?page=2>; rel=next",
            )),
        );
        assert!(github.next_page(&foreign).is_none());
        assert_eq!(
            linked_page("https://api.github.com/?page=2&page=3", "api.github.com"),
            None
        );
        let aws = workflow(json!({"operation":"awsRunningInstances"}));
        assert!(
            aws.next_page(&reply(
                "<DescribeInstancesResponse><nextToken>unsafe</nextToken>",
                None
            ))
            .is_none()
        );
        let cf = workflow(json!({"operation":"cloudflareDnsRecords"}));
        assert!(
            cf.next_page(&reply(
                r#"{"success":false,"result_info":{"page":1,"total_pages":2}}"#,
                None
            ))
            .is_none()
        );
        let oci = workflow(json!({"operation":"ociRunningInstances"}));
        let mut response = reply("[]", Some(("opc-next-page", "token")));
        response.truncated = true;
        assert!(oci.next_page(&response).is_none());
        response.truncated = false;
        response.status = 403;
        assert!(oci.next_page(&response).is_none());
    }
}
