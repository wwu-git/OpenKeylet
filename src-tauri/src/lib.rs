mod action_workflow;
pub mod api_catalog;
pub mod api_provider;
pub mod aws;
pub mod backup;
pub mod browser;
pub mod browser_vault;
pub mod crypto;
pub mod certificates;
mod env_template;
pub mod executor;
pub mod generator;
pub mod github_backup;
pub mod item_types;
pub mod login_uri;
pub mod mcp;
pub mod migration;
pub mod model;
mod native_workflow;
mod oci;
pub mod passkey;
pub mod policy;
pub mod runtime_epoch;
mod special_workflow;
pub mod storage;
pub mod terminal;
pub mod vault;
pub mod vault_write;
mod ssh_keys;
pub mod ssh_transport;

#[cfg(feature = "gui")]
pub mod agent_registry;
#[cfg(feature = "gui")]
mod clipboard;
#[cfg(feature = "gui")]
pub mod commands;
#[cfg(feature = "desktop-fill")]
pub mod desktop;
#[cfg(not(feature = "desktop-fill"))]
#[path = "desktop_headless.rs"]
pub mod desktop;
#[cfg(feature = "gui")]
pub mod gui_instance;
#[cfg(feature = "gui")]
pub mod system_integration;

#[cfg(feature = "gui")]
pub use commands::run_gui;
