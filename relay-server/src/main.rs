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
    ClientEvent, ClientFrame, PROTOCOL_VERSION, PresenceState, SecureChannel, ServerEvent,
    ServerFrame, decrypt_session_key, is_valid_pairing_code,
};
use tokio::sync::{RwLock, mpsc};
use tracing::{info, warn};

mod persistence;

const LAN_SERVICE_TYPE: &str = "_chicklink._tcp.local.";

type Tx = mpsc::UnboundedSender<ServerEvent>;
type Sessions = Arc<RwLock<HashMap<String, Tx>>>;

#[derive(Clone)]
struct AppState {
    sessions: Sessions,
    access_tokens: Option<Arc<HashMap<String, String>>>,
    pairing_codes: Arc<RwLock<HashMap<String, String>>>,
    partners: Arc<RwLock<HashMap<String, String>>>,
    /// Recipient -> requester. A relationship is kept until the recipient accepts.
    unpair_requests: Arc<RwLock<HashMap<String, String>>>,
    /// Offline owners receive the completed unpair result on their next connection.
    unpair_notifications: Arc<RwLock<HashMap<String, String>>>,
    display_names: Arc<RwLock<HashMap<String, String>>>,
    encryption_private_key: Option<Arc<RsaPrivateKey>>,
    relationship_store: persistence::Store,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let encryption_private_key = load_encryption_private_key()?;
    if env_flag("RELAY_REQUIRE_ENCRYPTION") && encryption_private_key.is_none() {
        anyhow::bail!("RELAY_REQUIRE_ENCRYPTION is set but no RSA private key was configured");
    }
    let access_tokens = load_access_tokens()?;
    if env_flag("RELAY_REQUIRE_PROVISIONED_TOKENS") && access_tokens.is_none() {
        anyhow::bail!(
            "RELAY_REQUIRE_PROVISIONED_TOKENS is set but no access-token file was configured"
        );
    }
    let relationship_store =
        persistence::Store::new(std::env::var_os("RELAY_STATE_PATH").map(Into::into));
    let saved_partners = relationship_store.load()?;
    let state = AppState {
        sessions: Arc::default(),
        access_tokens,
        pairing_codes: Arc::default(),
        partners: Arc::new(RwLock::new(saved_partners)),
        unpair_requests: Arc::default(),
        unpair_notifications: Arc::default(),
        display_names: Arc::default(),
        encryption_private_key,
        relationship_store,
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
    while let Ok(Some(Ok(message))) =
        tokio::time::timeout(std::time::Duration::from_secs(15), stream.next()).await
    {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
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
                if user_id.is_some() {
                    break;
                }
                if protocol_version != PROTOCOL_VERSION {
                    let _ = outbound.send(ServerEvent::Error {
                        code: "version_mismatch".into(),
                        message: "Update the desktop client".into(),
                    });
                    break;
                }
                let Some(id) = resolve_user_id(state.access_tokens.as_deref(), &access_token)
                else {
                    send_error(
                        &outbound,
                        "authentication_failed",
                        "Access token is invalid",
                    );
                    break;
                };
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
                if let Some(former_partner_id) =
                    state.unpair_notifications.write().await.remove(&id)
                {
                    let _ = outbound.send(ServerEvent::Unpaired { former_partner_id });
                    user_id = Some(id);
                    continue;
                }
                if let Some(partner_id) = state.partners.read().await.get(&id).cloned() {
                    let partner_display_name =
                        state.display_names.read().await.get(&partner_id).cloned();
                    let _ = outbound.send(ServerEvent::Paired {
                        partner_id: partner_id.clone(),
                        partner_display_name,
                        initiated_by_you: false,
                    });

                    let requests = state.unpair_requests.read().await;
                    if let Some(requester_id) = requests.get(&id).cloned() {
                        let requester_display_name =
                            state.display_names.read().await.get(&requester_id).cloned();
                        let _ = outbound.send(ServerEvent::UnpairRequested {
                            requester_id,
                            requester_display_name,
                        });
                    } else if requests
                        .iter()
                        .any(|(recipient, requester)| requester == &id && recipient == &partner_id)
                    {
                        let _ = outbound.send(ServerEvent::UnpairPending { partner_id });
                    }
                }
                sync_presence(&state, &id).await;
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
                let mut updated = partners.clone();
                updated.insert(id.clone(), partner_id.clone());
                updated.insert(partner_id.clone(), id.clone());
                if let Err(error) = state.relationship_store.save(&updated).await {
                    warn!(%error, "could not persist pairing");
                    send_error(&outbound, "storage_failed", "配对保存失败，请稍后重试");
                    continue;
                }
                *partners = updated;
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
                sync_presence(&state, id).await;
            }
            ClientEvent::RequestUnpair => {
                let Some(id) = &user_id else { continue };
                let Some(partner_id) = state.partners.read().await.get(id).cloned() else {
                    send_error(&outbound, "not_paired", "No paired partner to disconnect");
                    continue;
                };
                let mut requests = state.unpair_requests.write().await;
                if requests.get(id) == Some(&partner_id) {
                    drop(requests);
                    send_error(
                        &outbound,
                        "unpair_response_required",
                        "Please respond to your partner's disconnect request",
                    );
                    continue;
                }
                requests.retain(|recipient, requester| {
                    recipient != id
                        && requester != id
                        && recipient != &partner_id
                        && requester != &partner_id
                });
                requests.insert(partner_id.clone(), id.clone());
                drop(requests);

                let _ = outbound.send(ServerEvent::UnpairPending {
                    partner_id: partner_id.clone(),
                });
                if let Some(peer) = state.sessions.read().await.get(&partner_id) {
                    let requester_display_name = state.display_names.read().await.get(id).cloned();
                    let _ = peer.send(ServerEvent::UnpairRequested {
                        requester_id: id.clone(),
                        requester_display_name,
                    });
                }
            }
            ClientEvent::RespondUnpair { accept } => {
                let Some(id) = &user_id else { continue };
                let requester_id = state.unpair_requests.write().await.remove(id);
                let Some(requester_id) = requester_id else {
                    send_error(
                        &outbound,
                        "unpair_request_not_found",
                        "Disconnect request was not found",
                    );
                    continue;
                };
                if !is_partner(&state, id, &requester_id).await {
                    send_error(
                        &outbound,
                        "not_paired",
                        "Disconnect request is no longer valid",
                    );
                    continue;
                }

                if !accept {
                    let _ = outbound.send(ServerEvent::UnpairCancelled {
                        partner_id: requester_id.clone(),
                    });
                    if let Some(peer) = state.sessions.read().await.get(&requester_id) {
                        let _ = peer.send(ServerEvent::UnpairCancelled {
                            partner_id: id.clone(),
                        });
                    }
                    continue;
                }

                let mut partners = state.partners.write().await;
                let mut updated = partners.clone();
                updated.remove(id);
                updated.remove(&requester_id);
                if let Err(error) = state.relationship_store.save(&updated).await {
                    warn!(%error, "could not persist unpairing");
                    drop(partners);
                    state
                        .unpair_requests
                        .write()
                        .await
                        .insert(id.clone(), requester_id);
                    send_error(&outbound, "storage_failed", "解除配对保存失败，请稍后重试");
                    continue;
                }
                *partners = updated;
                drop(partners);
                state
                    .unpair_requests
                    .write()
                    .await
                    .retain(|recipient, requester| {
                        recipient != id
                            && requester != id
                            && recipient != &requester_id
                            && requester != &requester_id
                    });

                let _ = outbound.send(ServerEvent::Unpaired {
                    former_partner_id: requester_id.clone(),
                });
                if let Some(peer) = state.sessions.read().await.get(&requester_id) {
                    let _ = peer.send(ServerEvent::Unpaired {
                        former_partner_id: id.clone(),
                    });
                } else {
                    state
                        .unpair_notifications
                        .write()
                        .await
                        .insert(requester_id, id.clone());
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
        let partner_id = state.partners.read().await.get(&id).cloned();
        let mut sessions = state.sessions.write().await;
        if sessions
            .get(&id)
            .is_some_and(|current| current.same_channel(&outbound))
        {
            sessions.remove(&id);
            if let Some(peer) = partner_id
                .as_ref()
                .and_then(|partner| sessions.get(partner))
            {
                let _ = peer.send(ServerEvent::Presence {
                    user_id: id.clone(),
                    state: PresenceState::Offline,
                    activity: None,
                });
            }
        }
        warn!(user_id = %id, "client disconnected");
    }
    writer.abort();
}

async fn sync_presence(state: &AppState, id: &str) {
    let Some(partner_id) = state.partners.read().await.get(id).cloned() else {
        return;
    };
    let sessions = state.sessions.read().await;
    if let Some(me) = sessions.get(id) {
        let _ = me.send(ServerEvent::Presence {
            user_id: partner_id.clone(),
            state: if sessions.contains_key(&partner_id) {
                PresenceState::Online
            } else {
                PresenceState::Offline
            },
            activity: None,
        });
    }
    if let Some(peer) = sessions.get(&partner_id) {
        let _ = peer.send(ServerEvent::Presence {
            user_id: id.to_owned(),
            state: PresenceState::Online,
            activity: None,
        });
    }
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

fn load_access_tokens() -> anyhow::Result<Option<Arc<HashMap<String, String>>>> {
    let Some(path) = std::env::var_os("RELAY_ACCESS_TOKENS_PATH") else {
        return Ok(None);
    };
    let json = std::fs::read_to_string(&path)?;
    let tokens: HashMap<String, String> = serde_json::from_str(&json)
        .map_err(|error| anyhow::anyhow!("invalid access-token file at {:?}: {error}", path))?;
    if tokens.is_empty() {
        anyhow::bail!("access-token file at {:?} must not be empty", path);
    }
    for (token, user_id) in &tokens {
        if !(32..=512).contains(&token.len()) {
            anyhow::bail!("access tokens must contain 32-512 bytes");
        }
        if !is_valid_user_id(user_id) {
            anyhow::bail!("invalid user id in access-token file: {user_id:?}");
        }
    }
    info!(path = ?path, token_count = tokens.len(), "provisioned access-token authentication enabled");
    Ok(Some(Arc::new(tokens)))
}

fn resolve_user_id(
    access_tokens: Option<&HashMap<String, String>>,
    access_token: &str,
) -> Option<String> {
    match access_tokens {
        Some(tokens) => tokens.get(access_token).cloned(),
        None => access_token
            .strip_prefix("dev:")
            .filter(|user_id| is_valid_user_id(user_id))
            .map(str::to_owned),
    }
}

fn is_valid_user_id(user_id: &str) -> bool {
    !user_id.is_empty()
        && user_id.len() <= 64
        && user_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .ok()
        .as_deref()
        .is_some_and(|value| matches!(value, "1" | "true" | "TRUE" | "yes" | "YES"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use shared_protocol::{PetSkin, SharedPetAction, encrypt_session_key};
    use tokio_tungstenite::{connect_async, tungstenite::Message as WireMessage};
    type TestSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn send(socket: &mut TestSocket, channel: &SecureChannel, event: ClientEvent) {
        let frame = ClientFrame::Encrypted {
            frame: channel.encrypt(&event).unwrap(),
        };
        socket
            .send(WireMessage::text(serde_json::to_string(&frame).unwrap()))
            .await
            .unwrap();
    }

    async fn receive(socket: &mut TestSocket, channel: &SecureChannel) -> ServerEvent {
        let message = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let ServerFrame::Encrypted { frame } =
            serde_json::from_str(message.to_text().unwrap()).unwrap();
        channel.decrypt(&frame).unwrap()
    }

    async fn client(
        url: &str,
        key: &RsaPrivateKey,
        token: &str,
        code: &str,
    ) -> (TestSocket, SecureChannel) {
        let (mut socket, _) = connect_async(url).await.unwrap();
        let (channel, session_key) = SecureChannel::generate();
        let handshake = ClientFrame::SecureHandshake {
            encrypted_session_key: encrypt_session_key(&key.to_public_key(), &session_key).unwrap(),
        };
        socket
            .send(WireMessage::text(
                serde_json::to_string(&handshake).unwrap(),
            ))
            .await
            .unwrap();
        send(
            &mut socket,
            &channel,
            ClientEvent::Hello {
                protocol_version: shared_protocol::PROTOCOL_VERSION,
                access_token: token.into(),
                device_id: code.into(),
                display_name: Some(code.into()),
            },
        )
        .await;
        assert!(matches!(
            receive(&mut socket, &channel).await,
            ServerEvent::Ready { .. }
        ));
        send(
            &mut socket,
            &channel,
            ClientEvent::RegisterPairingCode { code: code.into() },
        )
        .await;
        loop {
            match receive(&mut socket, &channel).await {
                ServerEvent::PairingCodeRegistered { .. } => break,
                ServerEvent::Paired { .. } | ServerEvent::Presence { .. } => {}
                event => panic!("unexpected registration event: {event:?}"),
            }
        }
        (socket, channel)
    }

    #[tokio::test]
    async fn encrypted_pet_and_message_sync_is_bidirectional_without_echo() {
        let key = Arc::new(RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap());
        let token_a = "a".repeat(64);
        let token_b = "b".repeat(64);
        let state = AppState {
            sessions: Arc::default(),
            pairing_codes: Arc::default(),
            partners: Arc::default(),
            unpair_requests: Arc::default(),
            unpair_notifications: Arc::default(),
            display_names: Arc::default(),
            encryption_private_key: Some(key.clone()),
            relationship_store: persistence::Store::new(None),
            access_tokens: Some(Arc::new(HashMap::from([
                (token_a.clone(), "a".into()),
                (token_b.clone(), "b".into()),
            ]))),
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/ws", listener.local_addr().unwrap());
        let router = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(state);
        let server = tokio::spawn(async {
            axum::serve(listener, router).await.unwrap();
        });
        let (mut a, ca) = client(&url, &key, &token_a, "7KQ3M9XZ").await;
        let (mut b, cb) = client(&url, &key, &token_b, "8KQ3M9XZ").await;
        send(
            &mut a,
            &ca,
            ClientEvent::SharedPetAction {
                recipient_id: "b".into(),
                action: SharedPetAction::Pet,
            },
        )
        .await;
        assert!(
            matches!(receive(&mut a, &ca).await, ServerEvent::Error { code, .. } if code == "not_paired")
        );
        send(
            &mut a,
            &ca,
            ClientEvent::PairWithCode {
                code: "8KQ3M9XZ".into(),
            },
        )
        .await;
        assert!(matches!(
            receive(&mut a, &ca).await,
            ServerEvent::Paired { .. }
        ));
        assert!(matches!(
            receive(&mut b, &cb).await,
            ServerEvent::Paired { .. }
        ));
        assert!(matches!(receive(&mut a, &ca).await,
            ServerEvent::Presence { user_id, state: PresenceState::Online, .. } if user_id == "b"));
        assert!(matches!(receive(&mut b, &cb).await,
            ServerEvent::Presence { user_id, state: PresenceState::Online, .. } if user_id == "a"));
        send(
            &mut a,
            &ca,
            ClientEvent::SharedPetAction {
                recipient_id: "b".into(),
                action: SharedPetAction::Pet,
            },
        )
        .await;
        assert!(
            matches!(receive(&mut b, &cb).await, ServerEvent::SharedPetAction { sender_id, action: SharedPetAction::Pet } if sender_id == "a")
        );
        // A pong must be the next event: the relay must not echo A's own pat.
        send(&mut a, &ca, ClientEvent::Ping).await;
        assert!(matches!(receive(&mut a, &ca).await, ServerEvent::Pong));
        send(
            &mut b,
            &cb,
            ClientEvent::SharedPetAction {
                recipient_id: "a".into(),
                action: SharedPetAction::SetSkin {
                    skin: PetSkin::Blueberry,
                },
            },
        )
        .await;
        assert!(matches!(
            receive(&mut a, &ca).await,
            ServerEvent::SharedPetAction {
                action: SharedPetAction::SetSkin {
                    skin: PetSkin::Blueberry
                },
                ..
            }
        ));
        send(
            &mut b,
            &cb,
            ClientEvent::DirectMessage {
                recipient_id: "a".into(),
                body: shared_protocol::MessageBody::Text {
                    text: "hello".into(),
                },
            },
        )
        .await;
        assert!(
            matches!(receive(&mut a, &ca).await, ServerEvent::DirectMessage { sender_id, .. } if sender_id == "b")
        );
        send(
            &mut b,
            &cb,
            ClientEvent::SharedPetAction {
                recipient_id: "a".into(),
                action: SharedPetAction::Pet,
            },
        )
        .await;
        assert!(matches!(
            receive(&mut a, &ca).await,
            ServerEvent::SharedPetAction {
                action: SharedPetAction::Pet,
                ..
            }
        ));
        a.close(None).await.unwrap();
        assert!(matches!(receive(&mut b, &cb).await,
            ServerEvent::Presence { user_id, state: PresenceState::Offline, .. } if user_id == "a"));
        let (mut a, _ca) = client(&url, &key, &token_a, "7KQ3M9XZ").await;
        assert!(matches!(receive(&mut b, &cb).await,
            ServerEvent::Presence { user_id, state: PresenceState::Online, .. } if user_id == "a"));
        // A remains connected but stops heartbeats; B must stay online and learn A timed out.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(21);
        loop {
            assert!(
                tokio::time::Instant::now() < deadline,
                "missing idle disconnect notification"
            );
            send(&mut b, &cb, ClientEvent::Ping).await;
            match receive(&mut b, &cb).await {
                ServerEvent::Presence {
                    user_id,
                    state: PresenceState::Offline,
                    ..
                } => {
                    assert_eq!(user_id, "a");
                    break;
                }
                ServerEvent::Pong => {}
                event => panic!("unexpected heartbeat event: {event:?}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
        let _ = a.close(None).await;
        b.close(None).await.unwrap();
        server.abort();
    }

    #[test]
    fn development_tokens_are_available_only_without_a_provisioned_map() {
        assert_eq!(
            resolve_user_id(None, "dev:owner-a").as_deref(),
            Some("owner-a")
        );
        assert!(resolve_user_id(None, "owner-a").is_none());
        assert!(resolve_user_id(None, "dev:").is_none());
        assert!(resolve_user_id(None, "dev:owner/a").is_none());
    }

    #[test]
    fn provisioned_tokens_disable_development_identity_fallback() {
        let tokens = HashMap::from([("a".repeat(64), "owner-a".into())]);
        assert_eq!(
            resolve_user_id(Some(&tokens), &"a".repeat(64)).as_deref(),
            Some("owner-a")
        );
        assert!(resolve_user_id(Some(&tokens), "dev:owner-a").is_none());
    }
}
