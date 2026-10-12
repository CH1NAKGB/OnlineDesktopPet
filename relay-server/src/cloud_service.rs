use super::{AppState, Tx, cloud};
use shared_protocol::{
    ServerEvent,
    cloud::{CloudRequest, CloudResponse},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

pub struct Budget {
    start: Instant,
    buckets: HashMap<String, u32>,
}
impl Default for Budget {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            buckets: HashMap::new(),
        }
    }
}
impl Budget {
    fn allow(&mut self, key: String, limit: u32) -> bool {
        if self.start.elapsed() >= Duration::from_secs(60) {
            *self = Self::default();
        }
        if !self.buckets.contains_key(&key) && self.buckets.len() >= 4096 {
            return false;
        }
        let count = self.buckets.entry(key).or_default();
        if *count >= limit {
            return false;
        }
        *count += 1;
        true
    }
}

pub fn open_store() -> anyhow::Result<Option<Arc<cloud::Store>>> {
    let Some(path) = std::env::var_os("RELAY_CLOUD_DB") else {
        return Ok(None);
    };
    anyhow::ensure!(
        std::env::var_os("RELAY_RSA_PRIVATE_KEY_PATH").is_some(),
        "Cloud accounts require RSA encryption"
    );
    let key_path = std::env::var_os("RELAY_CLOUD_KEY_PATH")
        .ok_or_else(|| anyhow::anyhow!("Missing cloud storage key path"))?;
    let key: [u8; 32] = std::fs::read(key_path)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("Cloud storage key must be 32 raw random bytes"))?;
    Ok(Some(Arc::new(cloud::Store::open(
        &PathBuf::from(path),
        key,
    )?)))
}

pub enum Outcome {
    Authenticate(String),
    Logout,
    Handled,
}

pub async fn handle(
    state: &AppState,
    outbound: &Tx,
    user_id: Option<&str>,
    token: &mut Option<String>,
    request: CloudRequest,
    peer_ip: std::net::IpAddr,
) -> Outcome {
    let client_id = match &request {
        CloudRequest::SendMessage { client_id, .. } => Some(client_id.clone()),
        _ => None,
    };
    match process(state, outbound, user_id, token, request, peer_ip).await {
        Ok((response, outcome)) => {
            let _ = outbound.send(ServerEvent::Cloud { response });
            outcome
        }
        Err(error) => {
            let message = error.to_string();
            let response = if let Some(client_id) = client_id {
                let retryable =
                    message.starts_with("服务繁忙") || message.starts_with("请求过于频繁");
                CloudResponse::SendRejected {
                    client_id,
                    message,
                    retryable,
                }
            } else {
                CloudResponse::Error { message }
            };
            let _ = outbound.send(ServerEvent::Cloud { response });
            Outcome::Handled
        }
    }
}

async fn process(
    state: &AppState,
    outbound: &Tx,
    user_id: Option<&str>,
    token: &mut Option<String>,
    request: CloudRequest,
    peer_ip: std::net::IpAddr,
) -> anyhow::Result<(CloudResponse, Outcome)> {
    let store = state
        .cloud
        .clone()
        .ok_or_else(|| anyhow::anyhow!("此服务器尚未启用账号和云同步"))?;
    let authentication = matches!(
        request,
        CloudRequest::Register { .. }
            | CloudRequest::Login { .. }
            | CloudRequest::Recover { .. }
            | CloudRequest::Resume { .. }
    );
    let mut user = None;
    if let Some(value) = token.as_ref() {
        user = store.resolve_token(value)?;
    }
    let (bucket, limit) = if authentication {
        (format!("auth:{peer_ip}"), 30)
    } else {
        (
            format!(
                "user:{}",
                user.as_deref()
                    .ok_or_else(|| anyhow::anyhow!("请先登录账号"))?
            ),
            600,
        )
    };
    anyhow::ensure!(
        state
            .cloud_budget
            .lock()
            .map_err(|_| anyhow::anyhow!("服务繁忙"))?
            .allow(bucket, limit),
        "请求过于频繁，请一分钟后重试"
    );
    let _permit = state
        .cloud_gate
        .try_acquire()
        .map_err(|_| anyhow::anyhow!("服务繁忙，请稍后重试"))?;
    let auth_result = match request {
        CloudRequest::Register {
            username,
            password,
            device_id,
            legacy_token,
        } => {
            let legacy = if let Some(legacy_token) = legacy_token {
                let id = super::resolve_user_id(state.access_tokens.as_deref(), &legacy_token)
                    .ok_or_else(|| anyhow::anyhow!("旧身份迁移凭据无效"))?;
                anyhow::ensure!(
                    user_id.is_none_or(|current| current == id),
                    "请先退出当前账号再迁移"
                );
                Some(id)
            } else {
                None
            };
            let auth = tokio::task::spawn_blocking(move || {
                store.register(&username, &password, &device_id, legacy.as_deref())
            })
            .await??;
            if let Some(old) = state.sessions.write().await.remove(&auth.user_id) {
                for peer in old.0 {
                    if !peer.same_channel(outbound) {
                        peer.1.notify_one();
                    }
                }
            }
            auth
        }
        CloudRequest::Login {
            username,
            password,
            device_id,
        } => {
            tokio::task::spawn_blocking(move || store.login(&username, &password, &device_id))
                .await??
        }
        CloudRequest::Recover {
            username,
            recovery_code,
            new_password,
            device_id,
        } => {
            let auth = tokio::task::spawn_blocking(move || {
                store.recover(&username, &recovery_code, &new_password, &device_id)
            })
            .await??;
            if let Some(old) = state.sessions.write().await.remove(&auth.user_id) {
                for peer in old.0 {
                    if !peer.same_channel(outbound) {
                        peer.1.notify_one();
                    }
                }
            }
            auth
        }
        CloudRequest::Resume {
            token: value,
            device_id,
        } => match tokio::task::spawn_blocking(move || store.resume(&value, &device_id)).await? {
            Ok(auth) => auth,
            Err(error) if error.to_string() == "invalid session" => {
                *token = None;
                return Ok((CloudResponse::LoggedOut, Outcome::Logout));
            }
            Err(error) => return Err(error),
        },
        CloudRequest::Logout => {
            if let Some(value) = token.take() {
                store.revoke(&value)?;
            }
            return Ok((CloudResponse::LoggedOut, Outcome::Logout));
        }
        CloudRequest::Sync { after } => {
            let user = user.ok_or_else(|| anyhow::anyhow!("请先登录账号"))?;
            let (items, next_cursor, has_more) =
                tokio::task::spawn_blocking(move || store.history(&user, after)).await??;
            return Ok((
                CloudResponse::History {
                    items,
                    next_cursor,
                    has_more,
                },
                Outcome::Handled,
            ));
        }
        CloudRequest::SendMessage {
            client_id,
            recipient_id,
            body,
        } => {
            let user = user.ok_or_else(|| anyhow::anyhow!("请先登录账号"))?;
            let partners = state.partners.read().await;
            anyhow::ensure!(
                partners.get(&user) == Some(&recipient_id),
                "只能给已配对的主人发送消息"
            );
            anyhow::ensure!(
                store.is_claimed_legacy(&recipient_id)?,
                "对方尚未升级云账号；请双方登录新版后在待发送中重试"
            );
            let sender = user.clone();
            let recipient = recipient_id.clone();
            let item = tokio::task::spawn_blocking(move || {
                store.send_message(&sender, &recipient, &client_id, body)
            })
            .await??;
            drop(partners);
            let response = CloudResponse::Message { item };
            let sessions = state.sessions.read().await;
            for id in [&user, &recipient_id] {
                if let Some(peers) = sessions.get(id) {
                    for peer in &peers.0 {
                        if !peer.same_channel(outbound) {
                            let _ = peer.send(ServerEvent::Cloud {
                                response: response.clone(),
                            });
                        }
                    }
                }
            }
            // The origin also receives a durable acknowledgement; clients deduplicate by seq.
            return Ok((response, Outcome::Handled));
        }
        CloudRequest::PutSticker { name, image } => {
            let user = user.ok_or_else(|| anyhow::anyhow!("请先登录账号"))?;
            let owner = user.clone();
            let item =
                tokio::task::spawn_blocking(move || store.save_sticker(&owner, &name, image))
                    .await??;
            let response = CloudResponse::StickerSaved { item };
            if let Some(peers) = state.sessions.read().await.get(&user) {
                for peer in &peers.0 {
                    if !peer.same_channel(outbound) {
                        let _ = peer.send(ServerEvent::Cloud {
                            response: response.clone(),
                        });
                    }
                }
            }
            return Ok((response, Outcome::Handled));
        }
        CloudRequest::ListStickers => {
            let user = user.ok_or_else(|| anyhow::anyhow!("请先登录账号"))?;
            let items = tokio::task::spawn_blocking(move || store.stickers(&user)).await??;
            return Ok((CloudResponse::Stickers { items }, Outcome::Handled));
        }
    };
    *token = Some(auth_result.token.clone());
    let username = auth_result.username.clone();
    Ok((
        CloudResponse::Authenticated {
            user_id: auth_result.user_id,
            username: auth_result.username,
            token: auth_result.token,
            recovery_code: auth_result.recovery_code,
        },
        Outcome::Authenticate(username),
    ))
}
