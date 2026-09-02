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
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};
use shared_protocol::{
    ClientEvent, ClientFrame, PROTOCOL_VERSION, SecureChannel, ServerEvent, ServerFrame,
    decrypt_session_key, is_valid_pairing_code,
};
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
    display_names: Arc<RwLock<HashMap<String, String>>>,
    encryption_private_key: Option<Arc<RsaPrivateKey>>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let state = AppState {
        sessions: Arc::default(),
        pairing_codes: Arc::default(),
        partners: Arc::default(),
        display_names: Arc::default(),
        encryption_private_key: load_encryption_private_key()?,
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
    use std::collections::HashMap;

    use mdns_sd::{ServiceDaemon, ServiceInfo};

    pub fn announce(port: u16) -> anyhow::Result<ServiceDaemon> {
        let daemon = ServiceDaemon::new()?;
        let ip = local_ip_address::local_ip()?;
        let scheme = std::env::var("LAN_DISCOVERY_SCHEME").unwrap_or_else(|_| "ws".into());
        let host = std::env::var("LAN_DISCOVERY_HOST").unwrap_or_else(|_| ip.to_string());
        let advertised_port = std::env::var("LAN_DISCOVERY_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(port);
        let path = std::env::var("LAN_DISCOVERY_PATH").unwrap_or_else(|_| "/ws".into());
        let properties = HashMap::from([
            ("protocol".to_owned(), "1".to_owned()),
            ("scheme".to_owned(), scheme.clone()),
            ("host".to_owned(), host.clone()),
            ("port".to_owned(), advertised_port.to_string()),
            ("path".to_owned(), path.clone()),
        ]);
        let info = ServiceInfo::new(
            LAN_SERVICE_TYPE,
            "chick-relay",
            "chick-link-relay.local.",
            ip,
            advertised_port,
            properties,
        )?;
        daemon.register(info)?;
        tracing::info!(%ip, scheme = %scheme, host = %host, port = %advertised_port, path = %path, "LAN discovery announcement enabled");
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
    let secure_channel = match &state.encryption_private_key {
        Some(private_key) => {
            let Some(Ok(Message::Text(text))) = stream.next().await else {
                return;
            };
            let Ok(ClientFrame::SecureHandshake {
                encrypted_session_key,
            }) = serde_json::from_str(&text)
            else {
                warn!("rejected connection without encrypted handshake");
                return;
            };
            let Ok(session_key) = decrypt_session_key(private_key, &encrypted_session_key) else {
                warn!("rejected connection with invalid encrypted handshake");
                return;
            };
            Some(SecureChannel::new(session_key))
        }
        None => None,
    };
    let (outbound, mut receiver) = mpsc::unbounded_channel();
    let writer_channel = secure_channel.clone();
    let writer = tokio::spawn(async move {
        while let Some(event) = receiver.recv().await {
            let json = if let Some(channel) = &writer_channel {
                let Ok(frame) = channel.encrypt(&event) else {
                    continue;
                };
                serde_json::to_string(&ServerFrame::Encrypted { frame })
            } else {
                serde_json::to_string(&event)
            };
            let Ok(json) = json else {
                continue;
            };
            if sink.send(Message::Text(json.into())).await.is_err() {
                break;
            }
        }
    });

    let mut user_id: Option<String> = None;
    while let Some(Ok(Message::Text(text))) = stream.next().await {
        let event = if let Some(channel) = &secure_channel {
            let Ok(ClientFrame::Encrypted { frame }) = serde_json::from_str(&text) else {
                send_error(&outbound, "invalid_event", "Encrypted message required");
                continue;
            };
            channel.decrypt(&frame)
        } else {
            serde_json::from_str::<ClientEvent>(&text).map_err(|error| error.to_string())
        };
        let Ok(event) = event else {
            let _ = outbound.send(ServerEvent::Error {
                code: "invalid_event".into(),
                message: "Invalid or unauthenticated message".into(),
            });
            continue;
        };
        match event {
            ClientEvent::Hello {
                protocol_version,
                access_token,
                display_name,
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
                let display_name = display_name
                    .as_deref()
                    .and_then(shared_protocol::normalize_display_name)
                    .unwrap_or_else(|| id.clone());
                state
                    .display_names
                    .write()
                    .await
                    .insert(id.clone(), display_name);
                let _ = outbound.send(ServerEvent::Ready {
                    user_id: id.clone(),
                });
                if let Some(partner_id) = state.partners.read().await.get(&id).cloned() {
                    let partner_display_name =
                        state.display_names.read().await.get(&partner_id).cloned();
                    let _ = outbound.send(ServerEvent::Paired {
                        partner_id,
                        partner_display_name,
                        initiated_by_you: false,
                    });
                }
                user_id = Some(id);
            }
            ClientEvent::UpdateProfile { display_name } => {
                let Some(id) = &user_id else { continue };
                let Some(display_name) = shared_protocol::normalize_display_name(&display_name)
                else {
                    send_error(
                        &outbound,
                        "invalid_display_name",
                        "Display name must be 1-24 characters",
                    );
                    continue;
                };
                state
                    .display_names
                    .write()
                    .await
                    .insert(id.clone(), display_name.clone());
                if let Some(partner_id) = state.partners.read().await.get(id).cloned() {
                    if let Some(peer) = state.sessions.read().await.get(&partner_id) {
                        let _ = peer.send(ServerEvent::ProfileUpdated {
                            user_id: id.clone(),
                            display_name,
                        });
                    }
                }
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
                let names = state.display_names.read().await;
                let _ = outbound.send(ServerEvent::Paired {
                    partner_id: partner_id.clone(),
                    partner_display_name: names.get(&partner_id).cloned(),
                    initiated_by_you: true,
                });
                if let Some(peer) = state.sessions.read().await.get(&partner_id) {
                    let _ = peer.send(ServerEvent::Paired {
                        partner_id: id.clone(),
                        partner_display_name: names.get(id).cloned(),
                        initiated_by_you: false,
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
                    let sender_id = user_id.clone().unwrap();
                    let sender_display_name =
                        state.display_names.read().await.get(&sender_id).cloned();
                    let _ = peer.send(ServerEvent::DirectMessage {
                        sender_id,
                        sender_display_name,
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

fn load_encryption_private_key() -> anyhow::Result<Option<Arc<RsaPrivateKey>>> {
    let Some(path) = std::env::var_os("RELAY_RSA_PRIVATE_KEY_PATH") else {
        return Ok(None);
    };
    let pem = std::fs::read_to_string(&path)?;
    let private_key = RsaPrivateKey::from_pkcs8_pem(&pem)
        .map_err(|error| anyhow::anyhow!("invalid RSA private key at {:?}: {error}", path))?;
    if private_key.n().bits() < 2048 {
        anyhow::bail!("RSA private key at {:?} must be at least 2048 bits", path);
    }
    info!(path = ?path, "application-layer relay encryption enabled");
    Ok(Some(Arc::new(private_key)))
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
