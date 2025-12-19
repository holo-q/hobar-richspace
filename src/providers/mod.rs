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

pub use render::RenderState;

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
        tracing::trace!(workspace, "Getting render state");

        let provider_id = self.claims.get(&workspace)?;
        tracing::trace!(workspace, provider_id = %provider_id, "Workspace claimed by provider");

        let provider = self.providers.get(provider_id)?;
        let state = provider.render_states.get(&workspace);

        if state.is_some() {
            tracing::trace!(
                workspace,
                provider_id = %provider_id,
                dots = state.map(|s| s.dots.len()).unwrap_or(0),
                "Render state found"
            );
        } else {
            tracing::trace!(
                workspace,
                provider_id = %provider_id,
                "Provider claimed workspace but has no render state"
            );
        }

        state
    }

    /// Check if workspace is claimed by a provider
    pub fn is_claimed(&self, workspace: i32) -> bool {
        let claimed = self.claims.contains_key(&workspace);
        tracing::trace!(workspace, claimed, "Workspace claim check");
        claimed
    }

    /// Check if any provider is animating (needs 60fps tick)
    pub fn any_animating(&self) -> bool {
        let animating = self.providers.values().any(|p| {
            p.render_states.values().any(|s| s.animating)
        });

        if animating {
            let animating_providers: Vec<_> = self.providers.iter()
                .filter_map(|(id, p)| {
                    let workspaces: Vec<_> = p.render_states.iter()
                        .filter(|(_, s)| s.animating)
                        .map(|(ws, _)| *ws)
                        .collect();
                    if workspaces.is_empty() {
                        None
                    } else {
                        Some((id.clone(), workspaces))
                    }
                })
                .collect();

            tracing::trace!(
                animating_providers = ?animating_providers,
                "Providers with animating workspaces"
            );
        } else {
            tracing::trace!("No providers animating");
        }

        animating
    }

    /// Handle provider event
    pub fn handle_event(&mut self, event: ProviderEvent) {
        tracing::debug!(event = ?event, "Handling provider event");

        match event {
            ProviderEvent::Connected { provider_id } => {
                tracing::info!(
                    provider = %provider_id,
                    total_providers = self.providers.len() + 1,
                    "Provider connected to registry"
                );

                self.providers.insert(provider_id.clone(), ProviderConnection {
                    id: provider_id.clone(),
                    render_states: HashMap::new(),
                    signals: HashMap::new(),
                });

                tracing::debug!(
                    provider = %provider_id,
                    "Provider connection initialized"
                );
            }

            ProviderEvent::RenderUpdate { provider_id, workspace, state } => {
                if let Some(provider) = self.providers.get_mut(&provider_id) {
                    tracing::debug!(
                        provider = %provider_id,
                        workspace,
                        dots = state.dots.len(),
                        urgent = state.urgent,
                        animating = state.animating,
                        "Render update applied to registry"
                    );

                    provider.render_states.insert(workspace, state);

                    // Auto-claim workspace when provider starts rendering it
                    let was_claimed = self.claims.get(&workspace);
                    if let Some(prev_owner) = was_claimed {
                        if prev_owner != &provider_id {
                            tracing::info!(
                                workspace,
                                old_owner = %prev_owner,
                                new_owner = %provider_id,
                                "Workspace claim transferred"
                            );
                        }
                    } else {
                        tracing::info!(
                            workspace,
                            owner = %provider_id,
                            "Workspace claimed by provider"
                        );
                    }

                    self.claims.insert(workspace, provider_id);
                } else {
                    tracing::warn!(
                        provider = %provider_id,
                        workspace,
                        "Render update for unknown provider - ignoring"
                    );
                }
            }

            ProviderEvent::SignalsUpdate { provider_id, workspace, signals } => {
                if let Some(provider) = self.providers.get_mut(&provider_id) {
                    tracing::debug!(
                        provider = %provider_id,
                        workspace,
                        signals = ?signals,
                        "Signals update applied to registry"
                    );

                    provider.signals.insert(workspace, signals);
                } else {
                    tracing::warn!(
                        provider = %provider_id,
                        workspace,
                        "Signals update for unknown provider - ignoring"
                    );
                }
            }

            ProviderEvent::Disconnected { provider_id } => {
                let claimed_workspaces: Vec<_> = self.claims.iter()
                    .filter(|(_, pid)| *pid == &provider_id)
                    .map(|(ws, _)| *ws)
                    .collect();

                tracing::info!(
                    provider = %provider_id,
                    claimed_workspaces = ?claimed_workspaces,
                    total_providers = self.providers.len().saturating_sub(1),
                    "Provider disconnected from registry"
                );

                // Remove claims for this provider
                self.claims.retain(|_, pid| pid != &provider_id);
                self.providers.remove(&provider_id);

                tracing::debug!(
                    provider = %provider_id,
                    remaining_claims = self.claims.len(),
                    "Provider cleanup complete"
                );
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
    tracing::info!("Starting provider IPC listener thread");

    std::thread::spawn(move || {
        tracing::debug!("Provider listener thread spawned, creating tokio runtime");

        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                tracing::debug!("Tokio runtime created successfully");
                rt
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to create tokio runtime for provider listener");
                panic!("Failed to create tokio runtime: {}", e);
            }
        };

        rt.block_on(async move {
            tracing::info!("Provider listener tokio runtime started, entering run_listener");
            if let Err(e) = run_listener(event_tx).await {
                tracing::error!(error = %e, "Provider listener error");
            }
            tracing::warn!("Provider listener exited");
        });
    });
}

/// Run the provider listener (async entry point)
async fn run_listener(event_tx: glib::Sender<ProviderEvent>) -> anyhow::Result<()> {
    let path = socket_path();
    tracing::debug!(socket_path = %path.display(), "Initializing provider listener");

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        tracing::debug!(parent_dir = %parent.display(), "Creating socket parent directory");
        tokio::fs::create_dir_all(parent).await?;
    }

    // Remove stale socket
    tracing::debug!(socket_path = %path.display(), "Removing stale socket");
    let _ = tokio::fs::remove_file(&path).await;

    tracing::debug!(socket_path = %path.display(), "Binding unix listener");
    let listener = UnixListener::bind(&path)?;
    tracing::info!(socket = %path.display(), "Provider listener bound and ready");

    let mut connection_counter: u64 = 0;

    loop {
        tracing::trace!("Waiting for provider connection");
        match listener.accept().await {
            Ok((stream, addr)) => {
                connection_counter += 1;
                let conn_id = connection_counter;
                tracing::info!(
                    connection_id = conn_id,
                    addr = ?addr,
                    "Provider connection accepted"
                );

                let tx = event_tx.clone();
                tokio::spawn(async move {
                    tracing::debug!(connection_id = conn_id, "Spawning connection handler task");
                    if let Err(e) = handle_connection(stream, tx, conn_id).await {
                        tracing::warn!(
                            connection_id = conn_id,
                            error = %e,
                            "Provider connection error"
                        );
                    }
                    tracing::debug!(connection_id = conn_id, "Connection handler task exited");
                });
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to accept provider connection");
            }
        }
    }
}

/// Handle a single provider connection
async fn handle_connection(
    stream: UnixStream,
    event_tx: glib::Sender<ProviderEvent>,
    connection_id: u64,
) -> anyhow::Result<()> {
    use std::time::Instant;

    tracing::debug!(connection_id, "Connection handler started");
    let start = Instant::now();

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut provider_id: Option<String> = None;
    let mut message_count: u64 = 0;

    loop {
        line.clear();

        tracing::trace!(connection_id, provider_id = ?provider_id, "Waiting for message");
        let read_start = Instant::now();
        let bytes = reader.read_line(&mut line).await?;
        let read_elapsed = read_start.elapsed();

        if bytes == 0 {
            // Connection closed
            tracing::info!(
                connection_id,
                provider_id = ?provider_id,
                message_count,
                duration_ms = start.elapsed().as_millis(),
                "Provider connection closed (EOF)"
            );
            break;
        }

        if read_elapsed.as_millis() > 10 {
            tracing::debug!(
                connection_id,
                provider_id = ?provider_id,
                read_ms = read_elapsed.as_millis(),
                "Slow read detected"
            );
        }

        message_count += 1;
        tracing::trace!(
            connection_id,
            provider_id = ?provider_id,
            bytes,
            raw_line = %line.trim(),
            "Received raw message"
        );

        let parse_start = Instant::now();
        let msg: ProviderMessage = match serde_json::from_str(&line) {
            Ok(m) => {
                let parse_elapsed = parse_start.elapsed();
                if parse_elapsed.as_micros() > 100 {
                    tracing::debug!(
                        connection_id,
                        provider_id = ?provider_id,
                        parse_us = parse_elapsed.as_micros(),
                        "Slow JSON parse detected"
                    );
                }
                m
            }
            Err(e) => {
                tracing::error!(
                    connection_id,
                    provider_id = ?provider_id,
                    error = %e,
                    line = %line.trim(),
                    "Failed to parse provider message"
                );
                continue;
            }
        };

        tracing::debug!(
            connection_id,
            provider_id = ?provider_id,
            message = ?msg,
            "Parsed provider message"
        );

        let handle_start = Instant::now();
        match msg {
            ProviderMessage::Register { provider_id: pid, signals } => {
                tracing::info!(
                    connection_id,
                    provider_id = %pid,
                    signals = ?signals,
                    "Provider registered"
                );
                provider_id = Some(pid.clone());

                if event_tx.send(ProviderEvent::Connected { provider_id: pid }).is_err() {
                    tracing::error!(
                        connection_id,
                        provider_id = ?provider_id,
                        "Failed to send Connected event - main loop may have exited"
                    );
                    break;
                }
            }

            ProviderMessage::Render { workspace, state } => {
                if let Some(ref pid) = provider_id {
                    tracing::debug!(
                        connection_id,
                        provider_id = %pid,
                        workspace,
                        dots = state.dots.len(),
                        urgent = state.urgent,
                        animating = state.animating,
                        "Render update"
                    );
                    tracing::trace!(
                        connection_id,
                        provider_id = %pid,
                        workspace,
                        state = ?state,
                        "Full render state"
                    );

                    if event_tx.send(ProviderEvent::RenderUpdate {
                        provider_id: pid.clone(),
                        workspace,
                        state,
                    }).is_err() {
                        tracing::error!(
                            connection_id,
                            provider_id = ?provider_id,
                            "Failed to send RenderUpdate event - main loop may have exited"
                        );
                        break;
                    }
                } else {
                    tracing::warn!(
                        connection_id,
                        workspace,
                        "Received Render message before Register - ignoring"
                    );
                }
            }

            ProviderMessage::Signals { workspace, signals } => {
                if let Some(ref pid) = provider_id {
                    tracing::debug!(
                        connection_id,
                        provider_id = %pid,
                        workspace,
                        signals = ?signals,
                        "Signals update"
                    );

                    if event_tx.send(ProviderEvent::SignalsUpdate {
                        provider_id: pid.clone(),
                        workspace,
                        signals,
                    }).is_err() {
                        tracing::error!(
                            connection_id,
                            provider_id = ?provider_id,
                            "Failed to send SignalsUpdate event - main loop may have exited"
                        );
                        break;
                    }
                } else {
                    tracing::warn!(
                        connection_id,
                        workspace,
                        "Received Signals message before Register - ignoring"
                    );
                }
            }

            ProviderMessage::Disconnect => {
                tracing::info!(
                    connection_id,
                    provider_id = ?provider_id,
                    message_count,
                    duration_ms = start.elapsed().as_millis(),
                    "Provider requested disconnect"
                );
                break;
            }
        }

        let handle_elapsed = handle_start.elapsed();
        if handle_elapsed.as_millis() > 5 {
            tracing::warn!(
                connection_id,
                provider_id = ?provider_id,
                handle_ms = handle_elapsed.as_millis(),
                "Slow message handling detected"
            );
        }
    }

    // Send disconnect event
    if let Some(pid) = provider_id {
        tracing::info!(
            connection_id,
            provider_id = %pid,
            message_count,
            total_duration_ms = start.elapsed().as_millis(),
            "Sending disconnect event"
        );
        if event_tx.send(ProviderEvent::Disconnected { provider_id: pid }).is_err() {
            tracing::error!(
                connection_id,
                "Failed to send Disconnected event - main loop may have exited"
            );
        }
    } else {
        tracing::warn!(
            connection_id,
            message_count,
            "Connection closed without registration"
        );
    }

    Ok(())
}
