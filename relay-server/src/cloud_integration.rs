//! Encrypted WebSocket coverage for accounts, device fanout, and durable cloud data.

use super::*;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use rsa::rand_core::{OsRng, RngCore};
use shared_protocol::{
    SharedPetAction,
    cloud::{AuthResult, CloudBody, CloudMessage, CloudRequest, CloudResponse, ImageData},
    encrypt_session_key,
};
use std::{io::Cursor, net::IpAddr, path::PathBuf, time::Duration};
use tokio::{task::JoinHandle, time::timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message as WireMessage};

const WAIT: Duration = Duration::from_secs(10);
const PASSWORD: &str = "integration-test-password";
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let nonce = OsRng.next_u64();
        let path = std::env::temp_dir().join(format!(
            "chicklink-cloud-network-{}-{nonce:016x}",
            std::process::id()
        ));
        std::fs::create_dir(&path).expect("create isolated test directory");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        // Also clean up during assertion unwinding, without masking the original failure.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Relay {
    server: Option<JoinHandle<()>>,
    state: AppState,
    key: Arc<RsaPrivateKey>,
    url: String,
    directory: TempDirectory,
}

impl Relay {
    async fn start() -> Self {
        Self::with_legacy_tokens(HashMap::new()).await
    }

    async fn with_legacy_tokens(access_tokens: HashMap<String, String>) -> Self {
        let directory = TempDirectory::new();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind isolated loopback listener");
        let url = format!("ws://{}/ws", listener.local_addr().unwrap());
        let key = Arc::new(
            tokio::task::spawn_blocking(|| RsaPrivateKey::new(&mut OsRng, 2048))
                .await
                .expect("RSA generation task")
                .expect("generate isolated RSA key"),
        );
        let mut storage_key = [0; 32];
        OsRng.fill_bytes(&mut storage_key);
        let path = directory.0.join("cloud.sqlite3");
        let cloud = tokio::task::spawn_blocking(move || cloud::Store::open(&path, storage_key))
            .await
            .expect("cloud store initialization task")
            .expect("open isolated encrypted database");
        let state = AppState {
            sessions: Arc::default(),
            access_tokens: Some(Arc::new(access_tokens)),
            pairing_codes: Arc::default(),
            partners: Arc::default(),
            unpair_requests: Arc::default(),
            unpair_notifications: Arc::default(),
            display_names: Arc::default(),
            encryption_private_key: Some(key.clone()),
            relationship_store: persistence::Store::new(Some(directory.0.join("partners.json"))),
            cloud: Some(Arc::new(cloud)),
            cloud_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            cloud_budget: Arc::default(),
        };
        let router = Router::new()
            .route("/ws", get(ws_handler))
            .with_state(state.clone());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("serve test relay");
        });
        Self {
            server: Some(server),
            state,
            key,
            url,
            directory,
        }
    }

    async fn devices(&self, owner: &str, expected: usize) {
        timeout(WAIT, async {
            loop {
                let count = self
                    .state
                    .sessions
                    .read()
                    .await
                    .get(owner)
                    .map_or(0, |s| s.0.len());
                if count == expected {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("owner session count did not settle");
    }

    async fn overflow_device(&self, owner: &str) {
        let peer = {
            let sessions = self.state.sessions.read().await;
            let devices = &sessions.get(owner).expect("connected owner").0;
            assert_eq!(devices.len(), 1);
            devices[0].clone()
        };
        // On this test's current-thread runtime, no writer can drain between these sends.
        // Fill the real socket's queue directly instead of relying on OS TCP buffer sizes.
        for _ in 0..peer.0.max_capacity() {
            assert!(peer.send(ServerEvent::Pong).is_ok());
        }
        assert!(
            peer.send(ServerEvent::Pong).is_err(),
            "queue must reject overflow"
        );
        self.devices(owner, 0).await;
    }

    async fn shutdown(mut self) {
        timeout(WAIT, async {
            while !self.state.sessions.read().await.is_empty() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("WebSocket sessions did not close");
        let server = self.server.take().unwrap();
        server.abort();
        assert!(server.await.is_err_and(|error| error.is_cancelled()));
        let path = self.directory.0.clone();
        drop(self);
        assert!(
            !path.exists(),
            "temporary database directory was not removed"
        );
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        if let Some(server) = &self.server {
            server.abort();
        }
    }
}

struct Client {
    socket: Socket,
    channel: SecureChannel,
}

impl Client {
    async fn connect(relay: &Relay) -> Self {
        let (mut socket, _) = timeout(WAIT, connect_async(&relay.url))
            .await
            .expect("connect timed out")
            .expect("connect encrypted client");
        let (channel, session_key) = SecureChannel::generate();
        let handshake = ClientFrame::SecureHandshake {
            encrypted_session_key: encrypt_session_key(&relay.key.to_public_key(), &session_key)
                .expect("encrypt session key"),
        };
        socket
            .send(WireMessage::text(
                serde_json::to_string(&handshake).unwrap(),
            ))
            .await
            .expect("send encrypted handshake");
        Self { socket, channel }
    }

    async fn send(&mut self, event: ClientEvent) {
        let frame = ClientFrame::Encrypted {
            frame: self.channel.encrypt(&event).expect("encrypt client event"),
        };
        timeout(
            WAIT,
            self.socket
                .send(WireMessage::text(serde_json::to_string(&frame).unwrap())),
        )
        .await
        .expect("send timed out")
        .expect("send encrypted event");
    }

    async fn cloud(&mut self, request: CloudRequest) {
        self.send(ClientEvent::Cloud { request }).await;
    }

    async fn receive(&mut self) -> ServerEvent {
        let message = timeout(WAIT, self.socket.next())
            .await
            .expect("encrypted response timed out")
            .expect("connection closed before response")
            .expect("receive WebSocket response");
        let text = message.to_text().expect("expected encrypted text frame");
        let ServerFrame::Encrypted { frame } = serde_json::from_str(text)
            .unwrap_or_else(|_| panic!("expected encrypted server frame"));
        // Never include raw frames, auth responses, or deserialization errors in failures.
        self.channel
            .decrypt(&frame)
            .unwrap_or_else(|_| panic!("could not decrypt server event"))
    }

    async fn response(&mut self) -> CloudResponse {
        match self.receive().await {
            ServerEvent::Cloud { response } => response,
            _ => panic!("expected cloud response"),
        }
    }

    async fn authenticate(&mut self, request: CloudRequest) -> AuthResult {
        self.cloud(request).await;
        let auth = match self.response().await {
            CloudResponse::Authenticated {
                user_id,
                username,
                token,
                recovery_code,
            } => AuthResult {
                user_id,
                username,
                token,
                recovery_code,
            },
            _ => panic!("expected successful authentication"),
        };
        assert!(
            matches!(self.receive().await, ServerEvent::Ready { user_id } if user_id == auth.user_id),
            "authentication must be followed by Ready for the same owner"
        );
        auth
    }

    async fn register(&mut self, username: &str, device: &str) -> AuthResult {
        self.authenticate(CloudRequest::Register {
            username: username.into(),
            password: PASSWORD.into(),
            device_id: device.into(),
            legacy_token: None,
        })
        .await
    }

    async fn paired(&mut self, partner: &str, initiated: bool) {
        assert!(
            matches!(self.receive().await, ServerEvent::Paired {
            partner_id, initiated_by_you, ..
        } if partner_id == partner && initiated_by_you == initiated),
            "expected pairing notification"
        );
    }

    async fn presence(&mut self, owner: &str, online: bool) {
        assert!(
            matches!(self.receive().await, ServerEvent::Presence { user_id, state, .. }
            if user_id == owner && matches!((state, online),
                (PresenceState::Online, true) | (PresenceState::Offline, false))),
            "expected owner presence"
        );
    }

    async fn pong(&mut self) {
        self.send(ClientEvent::Ping).await;
        assert!(
            matches!(self.receive().await, ServerEvent::Pong),
            "unexpected event before Pong"
        );
    }

    async fn message(&mut self) -> CloudMessage {
        match self.response().await {
            CloudResponse::Message { item } => item,
            _ => panic!("expected durable message"),
        }
    }

    async fn send_message(
        &mut self,
        recipient: &str,
        client_id: &str,
        body: CloudBody,
    ) -> CloudMessage {
        self.cloud(CloudRequest::SendMessage {
            client_id: client_id.into(),
            recipient_id: recipient.into(),
            body: body.clone(),
        })
        .await;
        let item = self.message().await;
        assert_eq!(item.client_id, client_id);
        assert_eq!(item.recipient_id, recipient);
        assert_eq!(item.body, body);
        // A Pong immediately after the acknowledgement detects accidental origin fanout.
        self.pong().await;
        item
    }

    async fn history(&mut self, after: u64) -> Vec<CloudMessage> {
        self.cloud(CloudRequest::Sync { after }).await;
        match self.response().await {
            CloudResponse::History {
                items,
                next_cursor,
                has_more,
            } => {
                assert!(!has_more, "test history unexpectedly exceeded one page");
                assert_eq!(next_cursor, items.last().map_or(after, |item| item.seq));
                items
            }
            _ => panic!("expected cloud history"),
        }
    }

    async fn rejected(&mut self, request: CloudRequest) {
        self.cloud(request).await;
        assert!(
            matches!(self.response().await, CloudResponse::Error { .. }),
            "expected cloud request rejection"
        );
    }

    async fn close(mut self) {
        timeout(WAIT, self.socket.close(None))
            .await
            .expect("close timed out")
            .expect("close client");
    }

    async fn expect_closed(mut self) {
        timeout(WAIT, async {
            loop {
                match self.socket.next().await {
                    None | Some(Ok(WireMessage::Close(_))) => break,
                    Some(Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed))
                    | Some(Err(tokio_tungstenite::tungstenite::Error::Protocol(
                        tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                    ))) => break,
                    Some(Ok(WireMessage::Text(text))) => {
                        let ServerFrame::Encrypted { frame } = serde_json::from_str(&text)
                            .unwrap_or_else(|_| panic!("expected encrypted queued frame"));
                        assert!(matches!(self.channel.decrypt::<ServerEvent>(&frame), Ok(ServerEvent::Pong)),
                            "only queued Pongs may precede overflow disconnect");
                    }
                    _ => panic!("unexpected event during overflow disconnect"),
                }
            }
        }).await.expect("overflow must close the actual WebSocket");
    }

    async fn expect_revoked(mut self) {
        let next = timeout(WAIT, self.socket.next())
            .await
            .expect("revocation must close the actual WebSocket");
        assert!(
            matches!(next,
                None | Some(Ok(WireMessage::Close(_)))
                | Some(Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed))
                | Some(Err(tokio_tungstenite::tungstenite::Error::Protocol(
                    tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
                )))
            ),
            "revoked connection must close without any further application data"
        );
    }
}

fn login(username: &str, device: &str, password: &str) -> CloudRequest {
    CloudRequest::Login {
        username: username.into(),
        password: password.into(),
        device_id: device.into(),
    }
}

async fn direct_cloud(
    state: &AppState,
    peer_ip: IpAddr,
    user_id: Option<&str>,
    token: &mut Option<String>,
    request: CloudRequest,
) -> CloudResponse {
    let (sender, mut receiver) = mpsc::channel(1);
    let outbound = Tx(sender, Arc::new(tokio::sync::Notify::new()));
    timeout(
        WAIT,
        cloud_service::handle(state, &outbound, user_id, token, request, peer_ip),
    )
    .await
    .expect("direct cloud request timed out");
    match receiver
        .try_recv()
        .expect("cloud handler must send a response")
        .event
    {
        ServerEvent::Cloud { response } => response,
        _ => panic!("expected direct cloud response"),
    }
}

async fn pair_clients(a: &mut Client, a_id: &str, b: &mut Client, b_id: &str) {
    b.send(ClientEvent::RegisterPairingCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    assert!(
        matches!(b.receive().await, ServerEvent::PairingCodeRegistered { code } if code == "8KQ3M9XZ")
    );
    a.send(ClientEvent::PairWithCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    a.paired(b_id, true).await;
    b.paired(a_id, false).await;
    a.presence(b_id, true).await;
    b.presence(a_id, true).await;
}

fn assert_send_rejected(response: CloudResponse, expected_id: &str, expected_retryable: bool) {
    assert!(
        matches!(response, CloudResponse::SendRejected { client_id, message, retryable }
        if client_id == expected_id && retryable == expected_retryable && !message.is_empty()),
        "send rejection must preserve the client ID and distinguish transient from permanent failures"
    );
}

fn text_body(text: &str) -> CloudBody {
    CloudBody::Text { text: text.into() }
}

fn tiny_png() -> ImageData {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(1, 1)
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encode minimal PNG fixture");
    ImageData {
        mime: "image/png".into(),
        data_base64: BASE64.encode(bytes.into_inner()),
        width: 1,
        height: 1,
    }
}

#[tokio::test]
async fn encrypted_cloud_fanout_offline_sync_and_owner_isolation() {
    let relay = Relay::start().await;
    let mut a1 = Client::connect(&relay).await;
    let a = a1.register("owner-a", "a1").await;
    let mut b = Client::connect(&relay).await;
    let b_auth = b.register("owner-b", "b1").await;
    assert_ne!(a.user_id, b_auth.user_id);
    let mut a2 = Client::connect(&relay).await;
    let second = a2.authenticate(login("owner-a", "a2", PASSWORD)).await;
    assert_eq!(second.user_id, a.user_id);
    assert!(second.recovery_code.is_none());
    relay.devices(&a.user_id, 2).await;

    b.send(ClientEvent::RegisterPairingCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    assert!(
        matches!(b.receive().await, ServerEvent::PairingCodeRegistered { code } if code == "8KQ3M9XZ")
    );
    a1.send(ClientEvent::PairWithCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    a1.paired(&b_auth.user_id, true).await;
    a2.paired(&b_auth.user_id, true).await;
    b.paired(&a.user_id, false).await;
    a1.presence(&b_auth.user_id, true).await;
    a2.presence(&b_auth.user_id, true).await;
    b.presence(&a.user_id, true).await;
    assert_eq!(
        relay
            .state
            .relationship_store
            .load()
            .unwrap()
            .get(&a.user_id),
        Some(&b_auth.user_id)
    );

    a1.send(ClientEvent::SharedPetAction {
        recipient_id: b_auth.user_id.clone(),
        action: SharedPetAction::Pet,
    })
    .await;
    for client in [&mut a2, &mut b] {
        assert!(
            matches!(client.receive().await, ServerEvent::SharedPetAction {
            sender_id, action: SharedPetAction::Pet,
        } if sender_id == a.user_id)
        );
        client.pong().await;
    }
    a1.pong().await;

    let online = a1
        .send_message(&b_auth.user_id, "online-text", text_body("hello from a1"))
        .await;
    assert_eq!(online.sender_id, a.user_id);
    assert_eq!(a2.message().await, online);
    assert_eq!(b.message().await, online);

    let image = tiny_png();
    a1.cloud(CloudRequest::PutSticker {
        name: "tiny sticker".into(),
        image: image.clone(),
    })
    .await;
    let sticker = match a1.response().await {
        CloudResponse::StickerSaved { item } => item,
        _ => panic!("expected saved sticker"),
    };
    assert_eq!(sticker.name, "tiny sticker");
    assert_eq!(sticker.image, image);
    assert!(matches!(a2.response().await, CloudResponse::StickerSaved { item } if item == sticker));
    a1.pong().await;
    a2.pong().await;
    a2.cloud(CloudRequest::ListStickers).await;
    assert!(
        matches!(a2.response().await, CloudResponse::Stickers { items } if items == vec![sticker])
    );
    b.cloud(CloudRequest::ListStickers).await;
    assert!(matches!(b.response().await, CloudResponse::Stickers { items } if items.is_empty()));
    let picture = a1
        .send_message(&b_auth.user_id, "online-image", CloudBody::Image { image })
        .await;
    assert!(picture.seq > online.seq);
    assert_eq!(a2.message().await, picture);
    assert_eq!(b.message().await, picture);
    assert_eq!(a2.history(0).await, vec![online.clone(), picture.clone()]);

    a1.close().await;
    relay.devices(&a.user_id, 1).await;
    // A2 keeps owner A online: no Offline event may precede B's Pong.
    b.pong().await;
    a2.send(ClientEvent::SharedPetAction {
        recipient_id: b_auth.user_id.clone(),
        action: SharedPetAction::Pet,
    })
    .await;
    assert!(matches!(b.receive().await, ServerEvent::SharedPetAction {
        sender_id, action: SharedPetAction::Pet,
    } if sender_id == a.user_id));
    a2.pong().await;

    b.pong().await;
    relay.overflow_device(&b_auth.user_id).await;
    b.expect_closed().await;
    a2.presence(&b_auth.user_id, false).await;
    let offline_body = text_body("saved while B is offline");
    let missed = a2
        .send_message(&b_auth.user_id, "offline-retry", offline_body.clone())
        .await;
    assert!(missed.seq > picture.seq);
    let retry = a2
        .send_message(&b_auth.user_id, "offline-retry", offline_body)
        .await;
    assert_eq!(retry, missed);
    let expected = vec![online, picture.clone(), missed.clone()];
    assert_eq!(a2.history(0).await, expected);
    let store = relay.state.cloud.as_ref().unwrap().clone();
    let owner = a.user_id.clone();
    let stored = tokio::task::spawn_blocking(move || store.history(&owner, 0))
        .await
        .unwrap()
        .expect("read committed database history");
    assert_eq!(
        stored.0, expected,
        "retry must not create a duplicate database record"
    );
    assert!(!stored.2);

    let mut b = Client::connect(&relay).await;
    let resumed = b
        .authenticate(CloudRequest::Resume {
            token: b_auth.token,
            device_id: "b1".into(),
        })
        .await;
    assert_eq!(resumed.user_id, b_auth.user_id);
    b.paired(&a.user_id, false).await;
    b.presence(&a.user_id, true).await;
    a2.presence(&b_auth.user_id, true).await;
    assert_eq!(b.history(picture.seq).await, vec![missed.clone()]);
    assert!(b.history(missed.seq).await.is_empty());
    assert_eq!(b.history(0).await, expected);

    let mut c = Client::connect(&relay).await;
    c.rejected(CloudRequest::Sync { after: 0 }).await;
    let outsider = c.register("owner-c", "c1").await;
    assert_ne!(outsider.user_id, a.user_id);
    assert_ne!(outsider.user_id, b_auth.user_id);
    assert!(c.history(0).await.is_empty());
    assert!(c.history(picture.seq).await.is_empty());
    c.cloud(CloudRequest::ListStickers).await;
    assert!(matches!(c.response().await, CloudResponse::Stickers { items } if items.is_empty()));
    c.cloud(CloudRequest::SendMessage {
        client_id: "unauthorized".into(),
        recipient_id: b_auth.user_id.clone(),
        body: text_body("not paired"),
    })
    .await;
    assert!(
        matches!(c.response().await, CloudResponse::SendRejected {
        client_id, retryable: false, ..
    } if client_id == "unauthorized"),
        "unauthorized sends must be rejected permanently with their client ID"
    );
    assert_eq!(b.history(0).await, expected);

    c.close().await;
    b.close().await;
    a2.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn encrypted_password_recovery_revokes_every_old_device_token() {
    let relay = Relay::start().await;
    let mut a1 = Client::connect(&relay).await;
    let first = a1.register("recover-owner", "a1").await;
    let mut a2 = Client::connect(&relay).await;
    let second = a2
        .authenticate(login("recover-owner", "a2", PASSWORD))
        .await;
    assert_eq!(first.user_id, second.user_id);
    let recovery_code = first
        .recovery_code
        .as_ref()
        .expect("registration supplies recovery code");
    let mut recovered = Client::connect(&relay).await;
    let recovering = recovered
        .authenticate(login("recover-owner", "recovered", PASSWORD))
        .await;
    assert_eq!(recovering.user_id, first.user_id);
    relay.devices(&first.user_id, 3).await;
    let replacement_password = "replacement-test-password";
    let replacement = recovered
        .authenticate(CloudRequest::Recover {
            username: "recover-owner".into(),
            recovery_code: recovery_code.clone(),
            new_password: replacement_password.into(),
            device_id: "recovered".into(),
        })
        .await;
    assert_eq!(replacement.user_id, first.user_id);
    assert!(
        replacement
            .recovery_code
            .as_ref()
            .is_some_and(|code| code != recovery_code)
    );
    assert!(
        replacement.token != first.token
            && replacement.token != second.token
            && replacement.token != recovering.token
    );
    assert!(recovered.history(0).await.is_empty());

    // Idle old devices must be closed proactively, without sending another request.
    a1.expect_revoked().await;
    a2.expect_revoked().await;
    relay.devices(&first.user_id, 1).await;
    let store = relay.state.cloud.as_ref().unwrap();
    for token in [&first.token, &second.token, &recovering.token] {
        assert!(
            store
                .resolve_token(token)
                .expect("resolve revoked token")
                .is_none()
        );
    }
    assert!(
        store
            .resolve_token(&replacement.token)
            .expect("resolve recovered token")
            .is_some_and(|id| id == first.user_id)
    );

    let mut probe = Client::connect(&relay).await;
    for (token, device) in [
        (first.token, "a1"),
        (second.token, "a2"),
        (recovering.token, "recovered"),
    ] {
        probe
            .cloud(CloudRequest::Resume {
                token,
                device_id: device.into(),
            })
            .await;
        assert!(
            matches!(probe.response().await, CloudResponse::LoggedOut),
            "revoked saved credentials must explicitly log the client out"
        );
    }
    probe
        .rejected(login("recover-owner", "old-password-probe", PASSWORD))
        .await;
    probe
        .rejected(CloudRequest::Recover {
            username: "recover-owner".into(),
            recovery_code: recovery_code.clone(),
            new_password: "another-test-password".into(),
            device_id: "old-code-probe".into(),
        })
        .await;
    let login_result = probe
        .authenticate(login("recover-owner", "new-login", replacement_password))
        .await;
    assert_eq!(login_result.user_id, first.user_id);
    assert!(login_result.recovery_code.is_none());
    assert!(probe.history(0).await.is_empty());
    recovered.pong().await;

    probe.close().await;
    recovered.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn encrypted_legacy_claim_closes_other_legacy_device_and_preserves_pair() {
    let legacy_id = "legacy-owner";
    let legacy_token = "integration-only-legacy-token-000000000000";
    let relay =
        Relay::with_legacy_tokens(HashMap::from([(legacy_token.into(), legacy_id.into())])).await;
    let mut a1 = Client::connect(&relay).await;
    let mut a2 = Client::connect(&relay).await;
    for (client, device) in [(&mut a1, "legacy-a1"), (&mut a2, "legacy-a2")] {
        client
            .send(ClientEvent::Hello {
                protocol_version: PROTOCOL_VERSION,
                access_token: legacy_token.into(),
                device_id: device.into(),
                display_name: Some("Legacy owner".into()),
            })
            .await;
        assert!(
            matches!(client.receive().await, ServerEvent::Ready { user_id } if user_id == legacy_id)
        );
        client.pong().await;
    }
    relay.devices(legacy_id, 2).await;

    let mut b = Client::connect(&relay).await;
    let partner = b.register("migration-partner", "b1").await;
    b.send(ClientEvent::RegisterPairingCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    assert!(
        matches!(b.receive().await, ServerEvent::PairingCodeRegistered { code } if code == "8KQ3M9XZ")
    );
    a1.send(ClientEvent::PairWithCode {
        code: "8KQ3M9XZ".into(),
    })
    .await;
    for client in [&mut a1, &mut a2] {
        client.paired(&partner.user_id, true).await;
        client.presence(&partner.user_id, true).await;
        client.pong().await;
    }
    b.paired(legacy_id, false).await;
    b.presence(legacy_id, true).await;

    let claimed = a1
        .authenticate(CloudRequest::Register {
            username: "claimed-owner".into(),
            password: PASSWORD.into(),
            device_id: "legacy-a1".into(),
            legacy_token: Some(legacy_token.into()),
        })
        .await;
    assert_eq!(claimed.user_id, legacy_id);
    assert!(claimed.recovery_code.is_some());
    a1.paired(&partner.user_id, false).await;
    a1.presence(&partner.user_id, true).await;
    b.presence(legacy_id, true).await;
    a1.pong().await;
    assert_eq!(
        relay
            .state
            .relationship_store
            .load()
            .unwrap()
            .get(legacy_id),
        Some(&partner.user_id)
    );

    let message = b
        .send_message(
            legacy_id,
            "after-legacy-claim",
            text_body("only the claimed account receives this"),
        )
        .await;
    assert_eq!(a1.message().await, message);
    a2.expect_revoked().await;
    relay.devices(legacy_id, 1).await;
    assert_eq!(a1.history(0).await, vec![message]);
    b.pong().await;

    let mut old_login = Client::connect(&relay).await;
    old_login
        .send(ClientEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            access_token: legacy_token.into(),
            device_id: "legacy-reconnect".into(),
            display_name: None,
        })
        .await;
    assert!(
        matches!(old_login.receive().await, ServerEvent::Error { code, .. } if code == "authentication_failed")
    );
    old_login.rejected(CloudRequest::Sync { after: 0 }).await;
    old_login.close().await;
    a1.close().await;
    b.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn encrypted_rotated_tokens_close_before_cloud_or_ping() {
    let relay = Relay::start().await;
    let mut current = Client::connect(&relay).await;
    let owner = current.register("rotation-owner", "same-device").await;
    for event in [
        ClientEvent::Cloud {
            request: CloudRequest::Sync { after: 0 },
        },
        ClientEvent::Ping,
    ] {
        let mut replacement = Client::connect(&relay).await;
        let auth = replacement
            .authenticate(login("rotation-owner", "same-device", PASSWORD))
            .await;
        assert_eq!(auth.user_id, owner.user_id);
        // Same-device login rotates the token without Recover's proactive notification.
        // These requests therefore exercise validation before Cloud and Ping dispatch.
        current.send(event).await;
        current.expect_revoked().await;
        relay.devices(&owner.user_id, 1).await;
        replacement.pong().await;
        current = replacement;
    }
    current.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn encrypted_send_rejected_retries_same_id_without_duplicate_history() {
    let relay = Relay::start().await;
    let mut a = Client::connect(&relay).await;
    let sender = a.register("retry-sender", "a1").await;
    let mut b = Client::connect(&relay).await;
    let recipient = b.register("retry-recipient", "b1").await;
    pair_clients(&mut a, &sender.user_id, &mut b, &recipient.user_id).await;
    let client_id = "busy-then-retry";
    let body = text_body("one durable message after retry");
    let permit = relay
        .state
        .cloud_gate
        .acquire()
        .await
        .expect("hold cloud gate");
    a.cloud(CloudRequest::SendMessage {
        client_id: client_id.into(),
        recipient_id: recipient.user_id.clone(),
        body: body.clone(),
    })
    .await;
    assert_send_rejected(a.response().await, client_id, true);
    // Rejecting the operation must neither acknowledge delivery nor notify the recipient.
    a.pong().await;
    b.pong().await;
    drop(permit);
    assert!(a.history(0).await.is_empty());
    assert!(b.history(0).await.is_empty());

    a.cloud(CloudRequest::SendMessage {
        client_id: "invalid-body".into(),
        recipient_id: recipient.user_id.clone(),
        body: text_body(""),
    })
    .await;
    assert_send_rejected(a.response().await, "invalid-body", false);
    a.pong().await;
    b.pong().await;

    let saved = a
        .send_message(&recipient.user_id, client_id, body.clone())
        .await;
    assert_eq!(saved.sender_id, sender.user_id);
    assert_eq!(b.message().await, saved);
    b.pong().await;
    let retried = a.send_message(&recipient.user_id, client_id, body).await;
    assert_eq!(retried, saved);
    assert_eq!(b.message().await, saved);
    b.pong().await;
    assert_eq!(a.history(0).await, vec![saved.clone()]);
    assert_eq!(b.history(0).await, vec![saved.clone()]);
    assert!(a.history(saved.seq).await.is_empty());
    a.close().await;
    b.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn encrypted_mixed_legacy_cloud_chat_rejects_both_directions() {
    let legacy_id = "mixed-legacy-owner";
    let legacy_token = "integration-only-mixed-legacy-token-000000";
    let relay =
        Relay::with_legacy_tokens(HashMap::from([(legacy_token.into(), legacy_id.into())])).await;
    let mut legacy = Client::connect(&relay).await;
    legacy
        .send(ClientEvent::Hello {
            protocol_version: PROTOCOL_VERSION,
            access_token: legacy_token.into(),
            device_id: "legacy-device".into(),
            display_name: None,
        })
        .await;
    assert!(
        matches!(legacy.receive().await, ServerEvent::Ready { user_id } if user_id == legacy_id)
    );
    let mut account = Client::connect(&relay).await;
    let owner = account.register("mixed-cloud-owner", "cloud-device").await;
    pair_clients(&mut account, &owner.user_id, &mut legacy, legacy_id).await;

    account
        .cloud(CloudRequest::SendMessage {
            client_id: "cloud-to-legacy".into(),
            recipient_id: legacy_id.into(),
            body: text_body("must not pretend this was delivered"),
        })
        .await;
    assert_send_rejected(account.response().await, "cloud-to-legacy", false);
    account.pong().await;
    legacy.pong().await;
    assert!(account.history(0).await.is_empty());

    account
        .send(ClientEvent::DirectMessage {
            recipient_id: legacy_id.into(),
            body: shared_protocol::MessageBody::Text {
                text: "cloud sender cannot use legacy delivery".into(),
            },
        })
        .await;
    assert!(
        matches!(account.receive().await, ServerEvent::Error { code, .. } if code == "cloud_upgrade_required")
    );
    account.pong().await;
    legacy.pong().await;

    legacy
        .send(ClientEvent::DirectMessage {
            recipient_id: owner.user_id.clone(),
            body: shared_protocol::MessageBody::Text {
                text: "legacy sender cannot bypass cloud history".into(),
            },
        })
        .await;
    assert!(
        matches!(legacy.receive().await, ServerEvent::Error { code, .. } if code == "cloud_upgrade_required")
    );
    legacy.pong().await;
    account.pong().await;
    assert!(account.history(0).await.is_empty());
    legacy.close().await;
    account.close().await;
    relay.shutdown().await;
}

#[tokio::test]
async fn cloud_auth_budget_exhaustion_is_isolated_by_peer_ip() {
    let relay = Relay::start().await;
    let exhausted_ip = IpAddr::from([192, 0, 2, 1]);
    let other_ip = IpAddr::from([192, 0, 2, 2]);
    let mut token = None;
    // Invalid Resume is cheap but consumes the same auth bucket as Login/Register.
    for attempt in 0..30 {
        let response = direct_cloud(
            &relay.state,
            exhausted_ip,
            None,
            &mut token,
            CloudRequest::Resume {
                token: "invalid-test-token".into(),
                device_id: "budget-probe".into(),
            },
        )
        .await;
        assert!(
            matches!(response, CloudResponse::LoggedOut),
            "auth attempt {attempt} should reach credential validation before the limit"
        );
    }
    let register = CloudRequest::Register {
        username: "budget-owner".into(),
        password: PASSWORD.into(),
        device_id: "budget-device".into(),
        legacy_token: None,
    };
    let response = direct_cloud(
        &relay.state,
        exhausted_ip,
        None,
        &mut token,
        register.clone(),
    )
    .await;
    assert!(
        matches!(response, CloudResponse::Error { .. }),
        "31st auth request must be rate limited"
    );
    assert!(
        token.is_none(),
        "rate limited registration must not authenticate"
    );

    let mut other_token = None;
    let response = direct_cloud(&relay.state, other_ip, None, &mut other_token, register).await;
    assert!(
        matches!(response, CloudResponse::Authenticated { username, .. } if username == "budget-owner"),
        "the same registration must succeed from an independent IP"
    );
    assert!(other_token.is_some());
    let response = direct_cloud(
        &relay.state,
        exhausted_ip,
        None,
        &mut token,
        login("budget-owner", "another-device", PASSWORD),
    )
    .await;
    assert!(
        matches!(response, CloudResponse::Error { .. }),
        "another IP must not reset the exhausted bucket"
    );
    relay.shutdown().await;
}

#[tokio::test]
async fn cloud_unauthenticated_requests_do_not_spend_user_budget() {
    let relay = Relay::start().await;
    let peer_ip = IpAddr::from([192, 0, 2, 3]);
    let mut token = None;
    let response = direct_cloud(
        &relay.state,
        peer_ip,
        None,
        &mut token,
        CloudRequest::Register {
            username: "business-budget-owner".into(),
            password: PASSWORD.into(),
            device_id: "budget-device".into(),
            legacy_token: None,
        },
    )
    .await;
    let owner = match response {
        CloudResponse::Authenticated { user_id, .. } => user_id,
        _ => panic!("expected registered budget owner"),
    };
    let mut anonymous_token = None;
    let response = direct_cloud(
        &relay.state,
        peer_ip,
        None,
        &mut anonymous_token,
        CloudRequest::Sync { after: 0 },
    )
    .await;
    let unauthenticated_error = match response {
        CloudResponse::Error { message } => message,
        _ => panic!("anonymous history request must be rejected"),
    };
    // Use the same IP as the owner; unauthenticated traffic must not share their budget.
    for attempt in 0..601 {
        let response = direct_cloud(
            &relay.state,
            peer_ip,
            None,
            &mut anonymous_token,
            CloudRequest::Sync { after: 0 },
        )
        .await;
        assert!(
            matches!(response, CloudResponse::Error { message } if message == unauthenticated_error),
            "anonymous attempt {attempt} must fail authentication before consuming a budget"
        );
    }
    assert!(anonymous_token.is_none());
    for attempt in 0..600 {
        let response = direct_cloud(
            &relay.state,
            peer_ip,
            Some(&owner),
            &mut token,
            CloudRequest::Sync { after: 0 },
        )
        .await;
        assert!(
            matches!(response, CloudResponse::History { items, next_cursor: 0, has_more: false } if items.is_empty()),
            "authenticated request {attempt} must retain its full business allowance"
        );
    }
    let response = direct_cloud(
        &relay.state,
        peer_ip,
        Some(&owner),
        &mut token,
        CloudRequest::Sync { after: 0 },
    )
    .await;
    assert!(
        matches!(response, CloudResponse::Error { message } if message != unauthenticated_error),
        "the 601st authenticated request must hit the business limit"
    );
    let response = direct_cloud(
        &relay.state,
        peer_ip,
        Some(&owner),
        &mut token,
        CloudRequest::SendMessage {
            client_id: "rate-limited-send".into(),
            recipient_id: owner.clone(),
            body: text_body("rate limit must be reported as retryable"),
        },
    )
    .await;
    assert_send_rejected(response, "rate-limited-send", true);
    relay.shutdown().await;
}
