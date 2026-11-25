use crate::action_workflow::NextCall;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use uuid::Uuid;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum DesktopWorkflow {
    EnvImport,
    PasswordVaultImport,
    TwoFactorImageImport,
    TwoFactorBrowserImport,
    GithubBackupRestore,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
enum Interface {
    Desktop,
    BrowserExtension,
}

#[derive(Serialize, JsonSchema)]
pub(crate) struct DesktopInstructions {
    interface: Interface,
    path: &'static [&'static str],
    steps: &'static [&'static str],
}

impl DesktopWorkflow {
    pub(crate) fn instructions(self) -> DesktopInstructions {
        match self {
            Self::EnvImport => DesktopInstructions {
                interface: Interface::Desktop,
                path: &[
                    "Settings",
                    "Local data",
                    "Import vault",
                    "Environment · .env",
                ],
                steps: &[
                    "Choose the .env file and review the import preview.",
                    "Select the entries to import and confirm.",
                    "For a local command, copy the redacted .env template and pass it as envTemplate.",
                ],
            },
            Self::PasswordVaultImport => DesktopInstructions {
                interface: Interface::Desktop,
                path: &["Settings", "Local data", "Import vault"],
                steps: &[
                    "Select the export format and choose the vault export file.",
                    "Review the import preview and confirm the import.",
                ],
            },
            Self::TwoFactorImageImport => DesktopInstructions {
                interface: Interface::Desktop,
                path: &["Project editor", "TOTP", "Import QR image"],
                steps: &[
                    "Choose or paste an image and check the issuer and account in the preview.",
                    "Choose Use setup key and save the project.",
                ],
            },
            Self::TwoFactorBrowserImport => DesktopInstructions {
                interface: Interface::BrowserExtension,
                path: &["OpenKeylet extension", "Scan page 2FA QR"],
                steps: &[
                    "Open the page showing the QR code and unlock the extension.",
                    "Choose the matching login item and select Scan and preview.",
                    "Check the issuer, account, and destination, then select Confirm and save 2FA.",
                ],
            },
            Self::GithubBackupRestore => DesktopInstructions {
                interface: Interface::Desktop,
                path: &["Settings", "Encrypted GitHub backup", "Restore"],
                steps: &[
                    "Unlock OpenKeylet and open the connected backup.",
                    "Choose Restore and review the cloud snapshot preview.",
                    "Choose Restore & merge to apply that snapshot.",
                ],
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FillTarget {
    Browser,
    Desktop,
    Terminal,
}

impl FillTarget {
    pub(crate) fn next_call(
        self,
        item_id: Uuid,
        module: &str,
        session_id: Option<Uuid>,
        submit: bool,
    ) -> NextCall {
        NextCall {
            tool: "credential_fill",
            arguments: json!({
                "itemId": item_id,
                "module": module,
                "target": self,
                "sessionId": session_id,
                "submit": submit,
            }),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalRunPlan {
    #[serde(default)]
    pub(crate) command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stdin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) timeout_seconds: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) env_template: Option<String>,
}

impl TerminalRunPlan {
    pub(crate) fn next_call(&self) -> NextCall {
        NextCall {
            tool: "terminal_run",
            arguments: json!(self),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct TerminalOpenPlan {
    #[serde(skip_serializing_if = "Option::is_none")]
    program: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    command: Option<String>,
}

impl TerminalOpenPlan {
    pub(crate) fn next_call(&self) -> NextCall {
        NextCall {
            tool: "terminal_start",
            arguments: json!(self),
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(
    tag = "step",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum TerminalSessionStep {
    Read {
        wait_seconds: Option<u64>,
        until: Option<String>,
    },
    Write {
        text: String,
        #[serde(default)]
        submit: bool,
    },
    Stop,
}

impl TerminalSessionStep {
    pub(crate) fn next_call(self, session_id: Uuid) -> NextCall {
        match self {
            Self::Read {
                wait_seconds,
                until,
            } => NextCall {
                tool: "terminal_read",
                arguments: json!({"sessionId": session_id, "waitSeconds": wait_seconds, "until": until}),
            },
            Self::Write { text, submit } => NextCall {
                tool: "terminal_write",
                arguments: json!({"sessionId": session_id, "text": text, "submit": submit}),
            },
            Self::Stop => NextCall {
                tool: "terminal_stop",
                arguments: json!({"sessionId": session_id}),
            },
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) enum VaultWriteOperation {
    Create,
    Update,
}

impl VaultWriteOperation {
    pub(crate) fn next_call(&self) -> NextCall {
        NextCall {
            tool: "vault_write_capabilities",
            arguments: json!({}),
        }
    }

    pub(crate) fn write_tool(&self) -> &'static str {
        match self {
            Self::Create => "items_create",
            Self::Update => "items_update",
        }
    }
}
