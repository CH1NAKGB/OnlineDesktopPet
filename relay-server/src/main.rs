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
use shared_protocol::{ClientEvent, PROTOCOL_VERSION, ServerEvent, is_valid_pairing_code};
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

const LAN_SERVICE_TYPE: &str = "_chicklink._tcp.local.";

type Tx = mpsc::UnboundedSender<ServerEvent>;
type Sessions = Arc<RwLock<HashMap<String, Tx>>>;

#[derive(Clone)]
struct AppState {
    sessions: Sessions,
    pairing_codes: Arc<RwLock<HashMap<String, String>>>,
    partners: Arc<RwLock<HashMap<String, String>>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let state = AppState {
        sessions: Arc::default(),
        pairing_codes: Arc::default(),
        partners: Arc::default(),
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
            ClientEvent::RegisterPairingCode { code } => {
                let Some(id) = &user_id else { continue };
                if !is_valid_pairing_code(&code) {
                    send_error(
                        &outbound,
                        "invalid_pairing_code",
                        "Pairing code format is invalid",
                    );
                    continue;
                }
                let mut codes = state.pairing_codes.write().await;
                match codes.get(&code) {
                    Some(owner) if owner != id => {
                        send_error(
                            &outbound,
                            "pairing_code_conflict",
                            "Pairing code is already in use",
                        );
                    }
                    _ => {
                        codes.retain(|_, owner| owner != id);
                        codes.insert(code.clone(), id.clone());
                        let _ = outbound.send(ServerEvent::PairingCodeRegistered { code });
                    }
                }
            }
            ClientEvent::PairWithCode { code } => {
                let Some(id) = &user_id else { continue };
                let partner = state.pairing_codes.read().await.get(&code).cloned();
                let Some(partner_id) = partner.filter(|partner| partner != id) else {
                    send_error(
                        &outbound,
                        "pairing_code_not_found",
                        "Pairing code was not found",
                    );
                    continue;
                };
                let mut partners = state.partners.write().await;
                if partners.contains_key(id) || partners.contains_key(&partner_id) {
                    send_error(
                        &outbound,
                        "already_paired",
                        "One of these accounts is already paired",
                    );
                    continue;
                }
                partners.insert(id.clone(), partner_id.clone());
                partners.insert(partner_id.clone(), id.clone());
                drop(partners);
                let _ = outbound.send(ServerEvent::Paired {
                    partner_id: partner_id.clone(),
                });
                if let Some(peer) = state.sessions.read().await.get(&partner_id) {
                    let _ = peer.send(ServerEvent::Paired {
                        partner_id: id.clone(),
                    });
                }
            }
            ClientEvent::DirectMessage { recipient_id, body } => {
                if user_id.is_none() {
                    continue;
                }
                if !body.is_valid() {
                    let _ = outbound.send(ServerEvent::Error {
                        code: "invalid_message".into(),
                        message: "Message body exceeds the allowed limits".into(),
                    });
                    continue;
                }
                if !is_partner(
                    &state,
                    user_id.as_deref().unwrap_or_default(),
                    &recipient_id,
                )
                .await
                {
                    send_error(
                        &outbound,
                        "not_paired",
                        "Messages can only be sent to your paired partner",
                    );
                    continue;
                }
                if let Some(peer) = state.sessions.read().await.get(&recipient_id) {
                    let _ = peer.send(ServerEvent::DirectMessage {
                        sender_id: user_id.clone().unwrap(),
                        body,
                    });
                }
            }
            ClientEvent::SharedPetAction {
                recipient_id,
                action,
            } => {
                if let Some(sender_id) = &user_id {
                    if !is_partner(&state, sender_id, &recipient_id).await {
                        send_error(
                            &outbound,
                            "not_paired",
                            "Pet actions can only be sent to your paired partner",
                        );
                        continue;
                    }
                    if let Some(peer) = state.sessions.read().await.get(&recipient_id) {
                        let _ = peer.send(ServerEvent::SharedPetAction {
                            sender_id: sender_id.clone(),
                            action,
                        });
                    }
                }
            }
            ClientEvent::Presence {
                state: presence,
                activity,
            } => {
                if let Some(id) = &user_id {
                    let partner_id = state.partners.read().await.get(id).cloned();
                    if let Some(partner_id) = partner_id {
                        if let Some(peer) = state.sessions.read().await.get(&partner_id) {
                            let _ = peer.send(ServerEvent::Presence {
                                user_id: id.clone(),
                                state: presence,
                                activity,
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

fn send_error(outbound: &Tx, code: &str, message: &str) {
    let _ = outbound.send(ServerEvent::Error {
        code: code.into(),
        message: message.into(),
    });
}

async fn is_partner(state: &AppState, user_id: &str, recipient_id: &str) -> bool {
    state
        .partners
        .read()
        .await
        .get(user_id)
        .is_some_and(|partner| partner == recipient_id)
}
