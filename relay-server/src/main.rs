use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket},
    },
    response::IntoResponse,
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use shared_protocol::{ClientEvent, PROTOCOL_VERSION, ServerEvent};
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

const LAN_SERVICE_TYPE: &str = "_chicklink._tcp.local.";

type Tx = mpsc::UnboundedSender<ServerEvent>;
type Sessions = Arc<RwLock<HashMap<String, Tx>>>;

#[derive(Clone)]
struct AppState {
    sessions: Sessions,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let state = AppState {
        sessions: Arc::default(),
    };
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state);
    let address: SocketAddr = std::env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()?;
    info!(%address, "relay server listening");
    let _lan_daemon = std::env::var("LAN_DISCOVERY")
        .ok()
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE"))
        .then(|| lan::announce(address.port()))
        .transpose()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

mod lan {
    use super::LAN_SERVICE_TYPE;
    use mdns_sd::{ServiceDaemon, ServiceInfo};

    pub fn announce(port: u16) -> anyhow::Result<ServiceDaemon> {
        let daemon = ServiceDaemon::new()?;
        let ip = local_ip_address::local_ip()?;
        let properties = [("protocol", "1")];
        let info = ServiceInfo::new(
            LAN_SERVICE_TYPE,
            "chick-relay",
            "chick-link-relay.local.",
            ip,
            port,
            &properties[..],
        )?;
        daemon.register(info)?;
        tracing::info!(%ip, %port, "LAN discovery announcement enabled");
        Ok(daemon)
    }
}

async fn ws_handler(
    State(state): State<AppState>,
    ws: axum::extract::ws::WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| serve_socket(state, socket))
}

async fn serve_socket(state: AppState, socket: WebSocket) {
    let (mut sink, mut stream) = socket.split();
    let (outbound, mut receiver) = mpsc::unbounded_channel();
    let writer = tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let Ok(json) = serde_json::to_string(&event) else {
                continue;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    let mut user_id: Option<String> = None;
    while let Some(Ok(Message::Text(text))) = stream.next().await {
        let Ok(event) = serde_json::from_str::<ClientEvent>(&text) else {
            let _ = outbound.send(ServerEvent::Error {
                code: "invalid_event".into(),
                message: "Invalid JSON message".into(),
            });
            continue;
        };
        match event {
            ClientEvent::Hello {
                protocol_version,
                access_token,
                ..
            } => {
                if protocol_version != PROTOCOL_VERSION {
                    let _ = outbound.send(ServerEvent::Error {
                        code: "version_mismatch".into(),
                        message: "Update the desktop client".into(),
                    });
                    break;
                }
                // Development-only identity. Replace with JWT signature verification from the auth service.
                let id = access_token
                    .strip_prefix("dev:")
                    .unwrap_or("anonymous")
                    .to_owned();
                state
                    .sessions
                    .write()
                    .await
                    .insert(id.clone(), outbound.clone());
                let _ = outbound.send(ServerEvent::Ready {
                    user_id: id.clone(),
                });
                user_id = Some(id);
            }
            ClientEvent::DirectMessage { recipient_id, body } => {
                if user_id.is_none() {
                    continue;
                }
                if let Some(peer) = state.sessions.read().await.get(&recipient_id) {
                    let _ = peer.send(ServerEvent::DirectMessage {
                        sender_id: user_id.clone().unwrap(),
                        body,
                    });
                }
            }
            ClientEvent::Presence {
                state: presence,
                activity,
            } => {
                // Presence fan-out will become contact-list scoped once the auth/database service is added.
                if let Some(id) = &user_id {
                    for (peer_id, peer) in state.sessions.read().await.iter() {
                        if peer_id != id {
                            let _ = peer.send(ServerEvent::Presence {
                                user_id: id.clone(),
                                state: presence,
                                activity: activity.clone(),
                            });
                        }
                    }
                }
            }
            ClientEvent::Ping => {
                let _ = outbound.send(ServerEvent::Pong);
            }
        }
    }

    if let Some(id) = user_id {
        state.sessions.write().await.remove(&id);
        warn!(user_id = %id, "client disconnected");
    }
    writer.abort();
}
