use crate::model::StoredConnection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "camelCase", rename_all_fields = "camelCase", deny_unknown_fields)]
#[schemars(extend("type" = "object"))]
pub(crate) enum ActionCapabilitiesInput {
    SelectProject {
        capability: SelectableCapability,
        #[serde(default)]
        item: String,
        item_id: Option<Uuid>,
    },
    HttpRequest {
        #[serde(default)]
        item: String,
        item_id: Option<Uuid>,
        #[serde(default)]
        url: String,
        method: Option<String>,
        #[serde(default)]
        query: HashMap<String, Value>,
        body: Option<Value>,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SelectableCapability {
    Http,
    Fill,
}

impl SelectableCapability {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Fill => "fill",
        }
    }
}

#[derive(Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SelectionReason {
    Explicit,
    Session,
    Unique,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct ProjectCandidate {
    pub(crate) id: Uuid,
    pub(crate) name: String,
}

impl From<&StoredConnection> for ProjectCandidate {
    fn from(stored: &StoredConnection) -> Self {
        Self {
            id: stored.id,
            name: stored.name.clone(),
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct SelectedProject {
    #[serde(flatten)]
    candidate: ProjectCandidate,
    selection: SelectionReason,
}

impl SelectedProject {
    pub(crate) fn new(stored: &StoredConnection, selection: SelectionReason) -> Self {
        Self {
            candidate: stored.into(),
            selection,
        }
    }
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct MissingInput {
    pub(crate) name: &'static str,
    pub(crate) source: &'static str,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum UnavailableCode {
    ProjectNotFound,
    NoCompatibleProject,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NeedInputCode {
    MissingRequestUrl,
    InvalidRequestUrl,
    InvalidHttpMethod,
    MissingConfiguration,
    MissingSecretBinding,
    UnsupportedTarget,
}

#[derive(Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TargetSource {
    Saved,
    Relative,
    Explicit,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct HttpTarget {
    pub(crate) source: TargetSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) url: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct NextCall {
    pub(crate) tool: &'static str,
    pub(crate) arguments: Value,
}

#[derive(Serialize, JsonSchema)]
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[schemars(extend("type" = "object"))]
pub(crate) enum ActionResponse {
    Selected {
        project: SelectedProject,
    },
    Ready {
        project: SelectedProject,
        target: HttpTarget,
        next_call: NextCall,
    },
    NeedsInput {
        project: SelectedProject,
        code: NeedInputCode,
        missing_inputs: Vec<MissingInput>,
    },
    ChooseProject {
        candidates: Vec<ProjectCandidate>,
        missing_inputs: Vec<MissingInput>,
    },
    Unavailable {
        code: UnavailableCode,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        candidates: Vec<ProjectCandidate>,
    },
}

#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum HttpResponseFormat {
    #[default]
    Legacy,
    Structured,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HttpToolResponse {
    pub(crate) status: u16,
    pub(crate) status_text: String,
    pub(crate) url: String,
    pub(crate) headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) body: Option<String>,
    pub(crate) truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) saved_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bytes_transferred: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) project: Option<SelectedProject>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<HttpTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) error: Option<HttpRemoteError>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HttpRemoteError {
    pub(crate) code: &'static str,
    pub(crate) status: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
}
