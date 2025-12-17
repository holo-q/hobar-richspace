//! Provider IPC subsystem
//!
//! Enables external processes (like richspace-babel) to provide custom
//! workspace rendering. Providers connect via unix socket and push
//! render state updates that richspace draws via cairo.
//!
//! ## Architecture
//!
//! ```text
//! richspace-babel (provider process)
//!     │
//!     │ connect to $XDG_RUNTIME_DIR/richspace/providers.sock
//!     ▼
//! ProviderListener (this module)
//!     │
//!     │ receives JSON-lines: ProviderMessage
//!     ▼
//! AppEvent::ProviderUpdate
//!     │
//!     │ main thread handles update
//!     ▼
//! WorkspaceWidget renders provider's RenderState via cairo
//! ```
//!
//! ## Protocol
//!
//! Providers send JSON-lines messages:
//!
//! ```json
//! {"type": "register", "provider_id": "babel", "signals": {"has_claude": true}}
//! {"type": "render", "workspace": 1, "dots": [...], "urgent": false}
//! {"type": "signals", "workspace": 1, "has_claude": true, "claude_count": 2}
//! ```

mod render;

pub use render::{RenderState, RenderDot};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

/// Message types from providers
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderMessage {
    /// Provider registration (sent on connect)
    Register {
        provider_id: String,
        /// Initial signals this provider can emit
        #[serde(default)]
        signals: HashMap<String, serde_json::Value>,
    },

    /// Render state update for a workspace
    Render {
        workspace: i32,
        #[serde(flatten)]
        state: RenderState,
    },

    /// Provider signals for workspace matching
    /// Richspace uses these + its own wnck queries to decide
    /// which provider claims each workspace
    Signals {
        workspace: i32,
        /// Signal key-value pairs (e.g., "has_claude": true)
        #[serde(flatten)]
        signals: HashMap<String, serde_json::Value>,
    },

    /// Provider disconnecting cleanly
    Disconnect,
}

/// Event sent to main thread when provider state changes
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    /// Provider connected and registered
    Connected {
        provider_id: String,
    },
    /// Provider sent render update
    RenderUpdate {
        provider_id: String,
        workspace: i32,
        state: RenderState,
    },
    /// Provider sent signals update
    SignalsUpdate {
        provider_id: String,
        workspace: i32,
        signals: HashMap<String, serde_json::Value>,
    },
    /// Provider disconnected
    Disconnected {
        provider_id: String,
    },
}

/// Connected provider state
#[derive(Debug)]
pub struct ProviderConnection {
    pub id: String,
    /// Per-workspace render state
    pub render_states: HashMap<i32, RenderState>,
    /// Per-workspace signals
    pub signals: HashMap<i32, HashMap<String, serde_json::Value>>,
}

/// Provider registry - tracks all connected providers
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    /// Connected providers (provider_id → connection)
    pub providers: HashMap<String, ProviderConnection>,
    /// Workspace → provider_id claim mapping
    pub claims: HashMap<i32, String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get render state for a workspace (from claiming provider)
    pub fn get_render_state(&self, workspace: i32) -> Option<&RenderState> {
        let provider_id = self.claims.get(&workspace)?;
        let provider = self.providers.get(provider_id)?;
        provider.render_states.get(&workspace)
    }

    /// Check if workspace is claimed by a provider
    pub fn is_claimed(&self, workspace: i32) -> bool {
        self.claims.contains_key(&workspace)
    }

    /// Check if any provider is animating (needs 60fps tick)
    pub fn any_animating(&self) -> bool {
        self.providers.values().any(|p| {
            p.render_states.values().any(|s| s.animating)
        })
    }

    /// Handle provider event
    pub fn handle_event(&mut self, event: ProviderEvent) {
        match event {
            ProviderEvent::Connected { provider_id } => {
                tracing::info!(provider = %provider_id, "Provider connected");
                self.providers.insert(provider_id.clone(), ProviderConnection {
                    id: provider_id,
                    render_states: HashMap::new(),
                    signals: HashMap::new(),
                });
            }

            ProviderEvent::RenderUpdate { provider_id, workspace, state } => {
                if let Some(provider) = self.providers.get_mut(&provider_id) {
                    provider.render_states.insert(workspace, state);
                    // Auto-claim workspace when provider starts rendering it
                    self.claims.insert(workspace, provider_id);
                }
            }

            ProviderEvent::SignalsUpdate { provider_id, workspace, signals } => {
                if let Some(provider) = self.providers.get_mut(&provider_id) {
                    provider.signals.insert(workspace, signals);
                }
            }

            ProviderEvent::Disconnected { provider_id } => {
                tracing::info!(provider = %provider_id, "Provider disconnected");
                // Remove claims for this provider
                self.claims.retain(|_, pid| pid != &provider_id);
                self.providers.remove(&provider_id);
            }
        }
    }
}

/// Get provider socket path
pub fn socket_path() -> PathBuf {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("richspace").join("providers.sock")
}

/// Start the provider IPC listener
///
/// Runs in a tokio runtime on a background thread.
/// Sends ProviderEvents to the provided sender.
pub fn start_listener(event_tx: glib::Sender<ProviderEvent>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to create tokio runtime");

        rt.block_on(async move {
            if let Err(e) = run_listener(event_tx).await {
                tracing::error!(error = %e, "Provider listener error");
            }
        });
    });
}

/// Run the provider listener (async entry point)
async fn run_listener(event_tx: glib::Sender<ProviderEvent>) -> anyhow::Result<()> {
    let path = socket_path();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Remove stale socket
    let _ = tokio::fs::remove_file(&path).await;

    let listener = UnixListener::bind(&path)?;
    tracing::info!(socket = %path.display(), "Provider listener started");

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let tx = event_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, tx).await {
                        tracing::warn!(error = %e, "Provider connection error");
                    }
                });
            }
            Err(e) => {
                tracing::error!(error = %e, "Accept error");
            }
        }
    }
}

/// Handle a single provider connection
async fn handle_connection(
    stream: UnixStream,
    event_tx: glib::Sender<ProviderEvent>,
) -> anyhow::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut provider_id: Option<String> = None;

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            // Connection closed
            break;
        }

        let msg: ProviderMessage = match serde_json::from_str(&line) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, line = %line.trim(), "Invalid message");
                continue;
            }
        };

        match msg {
            ProviderMessage::Register { provider_id: pid, .. } => {
                provider_id = Some(pid.clone());
                event_tx.send(ProviderEvent::Connected { provider_id: pid }).ok();
            }

            ProviderMessage::Render { workspace, state } => {
                if let Some(ref pid) = provider_id {
                    event_tx.send(ProviderEvent::RenderUpdate {
                        provider_id: pid.clone(),
                        workspace,
                        state,
                    }).ok();
                }
            }

            ProviderMessage::Signals { workspace, signals } => {
                if let Some(ref pid) = provider_id {
                    event_tx.send(ProviderEvent::SignalsUpdate {
                        provider_id: pid.clone(),
                        workspace,
                        signals,
                    }).ok();
                }
            }

            ProviderMessage::Disconnect => {
                break;
            }
        }
    }

    // Send disconnect event
    if let Some(pid) = provider_id {
        event_tx.send(ProviderEvent::Disconnected { provider_id: pid }).ok();
    }

    Ok(())
}
