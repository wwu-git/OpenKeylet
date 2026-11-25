use crate::model::StoredConnection;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
#[serde(tag = "status", rename_all = "camelCase", rename_all_fields = "camelCase")]
#[schemars(extend("type" = "object"))]
pub(crate) enum ActionResponse {
    Selected {
        project: SelectedProject,
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
