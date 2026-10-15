# Chick Link

Chick Link 是一个面向 macOS 与 Windows 的轻量联机桌宠：两个人共养一只小鸡，可拖动、抚摸、聊天和查看对方状态。小鸡会偶尔在桌面散步。

客户端使用 Rust 原生窗口和 wgpu，不依赖 WebView；活动时按显示器刷新率调度，空闲时降低唤醒频率。macOS 与 Windows 共用协议和核心交互逻辑。

GitHub 最新公开稳定版：[v0.3.4](https://github.com/CH1NAKGB/OnlineDesktopPet/releases/tag/v0.3.4)。当前源代码包含尚未发布的 Chick Link 更新。
新对话或新开发者先读 [offread.md 接手文档](offread.md)；部署直接看 [快速部署](deploy/QUICKSTART.md)，客户端验收看 [测试说明](packaging/TESTING.md)。

> 状态边界：当前工作区包含尚未发布的账号、恢复码、云端加密历史、云表情和本机待发送队列开发改动。这些改动尚未替换线上服务，线上账号库目前也未启用。GitHub Release v0.3.4 仍是可下载稳定版。

## 已有功能

- macOS / Windows 透明桌宠窗口，支持拖动、抚摸、换肤和低频桌面走动。
- 配对码配对，配对成功双方都会收到反馈；解除配对需要双方确认。
- 独立聊天窗口、快捷回复、本地历史、正文气泡和 DPI 自适应。
- 在线、离线、暂离、网络质量和 RTT 显示。
- 柔和拍拍音效和消息音效，可选择音色并关闭声音。
- macOS LaunchAgent / Windows 当前用户 Run 项开机自启。
- 局域网 mDNS 搜索，未发现时回退到公网中继。

## 开发中的账号线

未发布开发线已经完成本地协议、存储和回环联机测试，包含用户名密码、一次性恢复码、多设备登录、加密 SQLite 聊天历史、PNG/JPEG 表情库、离线同步和持久待发送队列。服务端数据库加密不是端到端加密，服务端仍能在处理请求时解密内容；启用前请阅读 [云账号部署说明](deploy/standalone/CLOUD-ACCOUNTS.md)。

当前工作区需要重新完成回归；真机视觉验收和正式发布包验收仍需单独完成。

## 当前结构

```
pet-client/       原生桌宠、聊天 UI；macOS wgpu / Windows layered-window 渲染
relay-server/     WebSocket 在线与点对点消息中继
shared-protocol/  两端共享的序列化事件
pet-client/assets/ 字体与共享资源
```

## 开发启动

先安装稳定版 Rust：<https://rustup.rs/>。随后在仓库根目录运行：

```bash
cargo run -p relay-server
cargo run -p pet-client
```

客户端默认连接配置的正式中继地址。启动本地测试时，请同时设置 `CHICK_RELAY_URL=ws://127.0.0.1:8080/ws` 与不同的 `CHICK_ACCESS_TOKEN=dev:<name>`；该开发身份模式仅用于本机或可信局域网。

如果改为 `wss://`，客户端默认信任系统证书。没有公网域名时，也可以通过 `CHICK_TLS_CA_CERT_PATH=/path/to/ca-or-cert.pem` 额外信任一份 PEM 证书或私有 CA 链。

## 帧率策略

- 活跃状态（拖拽、抚摸、收到消息）：按当前显示器刷新率调度，范围 30–240Hz；wgpu 使用 AutoVsync，Windows 桌宠走系统 layered-window 合成。
- 空闲状态：50ms 唤醒一次，约 20FPS，减少透明桌宠的 GPU/CPU 开销。
- 动画逻辑必须使用时间差，不应把角色速度绑定到 60 或 120 FPS。

桌宠使用原生透明窗口渲染，不经过 WebView；小鸡会在桌面范围内低频随机走动。60/120Hz 动画一致性已有逻辑测试，但不等于所有硬件都已实测跑满。

左键拖动会使用系统原生窗口移动；快速轻点小鸡会触发短暂的开心跳动。按 `Esc` 可退出测试窗口。

## 共同宠物与交互

一只小鸡由两位主人共同操作。macOS 与 Windows 都可以右键小鸡显示原生交互菜单，提供抚摸、换肤、配对/聊天和退出等操作。Windows 桌宠不会显示在任务栏，并使用 `UpdateLayeredWindow` 与预乘逐像素 Alpha 渲染。

客户端每三秒测量一次 RTT，并在 macOS 菜单、Windows 托盘菜单和桌宠右键菜单中显示连接质量；短暂断线后会自动重连。每台端侧首次运行会生成并持久化 8 位配对码，连接中继后上报码值并查重。任意一端在“配对 / 聊天”中输入对方配对码后，服务端会原子绑定双方；发起端和接收端都会自动打开聊天、显示成功提示，无需另一端重复输入。只有已配对双方能互发消息、状态与共同宠物操作：

```bash
CHICK_RELAY_URL=ws://127.0.0.1:8080/ws CHICK_ACCESS_TOKEN=dev:owner-a cargo run -p pet-client
```

另一台电脑使用 `CHICK_ACCESS_TOKEN=dev:owner-b`，并把 `CHICK_RELAY_URL` 指向同一个中继即可验证同步。正式专用中继会持久化配对关系；“配对 / 聊天”会打开独立非透明 egui 窗口，提供本机配对码、历史气泡列表、快捷回复和多行发送区。未发布账号线还提供云端账号、图片表情、离线历史和本机待发送队列。解除共同养鸡关系必须由发起方先确认，再由另一位主人确认。设置区还可选择开机自启：macOS 使用当前用户的 LaunchAgent，Windows 使用当前用户的注册表 Run 项，均无需管理员权限。收到在线文本时会在桌宠上方显示正文气泡，Windows 气泡和桌宠会随系统 DPI 动态缩放。

## 局域网联机与搜索

在一台 macOS、Windows 或 Linux 电脑上启动 `relay-server`，并设置 `LAN_DISCOVERY=true`。它会通过 mDNS 广播 `_chicklink._tcp.local.`；同一 Wi-Fi 或有线网段内的客户端会优先搜索这个中继，并连接广播中声明的 `ws://` 或 `wss://` 地址。搜索最多等待一秒，未发现时自动回退到配置的公网地址。

```bash
LAN_DISCOVERY=true cargo run -p relay-server
```

局域网模式需要系统防火墙允许 UDP 5353（mDNS）与 TCP 8080（开发期中继端口）。企业/校园 Wi-Fi 若开启了“客户端隔离”，设备彼此不可见，这是网络策略限制，客户端会自动回退到公网连接。

如果加密入口并不直接跑在中继进程上，而是由局域网内的反向代理终止 TLS，可额外设置：

```bash
LAN_DISCOVERY=true
LAN_DISCOVERY_SCHEME=wss
LAN_DISCOVERY_HOST=192.168.1.20
LAN_DISCOVERY_PORT=443
LAN_DISCOVERY_PATH=/ws
```

这样客户端发现的就是加密入口，而不是中继容器的明文端口。

### Windows 真机验证

Windows 电脑安装 Rust 稳定版后，在同一仓库目录执行以下命令。中继可运行在 Mac 或 Windows 任一端，但只有运行中继的那台机器需要设置 `LAN_DISCOVERY=true`。

```powershell
cargo check --workspace
cargo run -p pet-client
```

若 Windows Defender 防火墙弹出提示，允许专用网络中的 `pet-client`。若中继也运行在 Windows 上，再额外允许 TCP 8080 和 UDP 5353。测试成功标准是：Windows 客户端在一秒内发现 `chick-relay`，并在中继日志中出现连接记录；关闭局域网中继后，客户端应回退到配置的公网 WebSocket 地址。

## 隐私边界

当前客户端上线发送 Online 且 activity 为 None；协议保留在线/离开/离线状态与 `SharedActivity`，但尚未实现活动应用采集或“分享当前应用”的设置入口。未来实现必须先获得用户明确启用，仅允许应用显示名，不上传窗口标题、文件路径、命令行、屏幕内容或进程列表。服务端已有配对权限检查；完整账号/好友管理仍未实现。本地聊天历史为明文 JSONL。

## 部署建议

当前适用方案为公网 TCP 18080 的加密中继；阿里云实际部署尚未完成。认证站、媒体存储与 NAS 备份是后续方向，不是现有已部署服务。用户目前不需要同步 NAS。

以下为旧域名 TLS 参考配置，仅在宿主机 80/443 可用时考虑。它目前没有接入预置令牌表，不能只加 TLS 就作为已加固的公网部署。将 `deploy/.env.example` 复制为 `deploy/.env`，填写域名后可在隔离环境从仓库根目录运行：

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml up -d --build
```

域名 DNS 需指向主机，且 80/443 可入站访问，Caddy 才能完成对应证书申请。用户的这三个常用端口已被占用，当前应使用下面的 18080 方案。

### 无域名、无 HTTPS 证书的加密通信

服务端保管 RSA 私钥，仅将公钥通过可信渠道分发给客户端后，可以使用 `ws://IP:端口/ws`：RSA-OAEP 用于保护 AES-256-GCM 会话密钥，后续业务事件以认证密文传输。这是客户端到中继的加密，不是两用户之间的端到端加密，服务器能解密消息。以下手动命令仅演示开发模式，公网请使用强制随机令牌的部署入口。

在运行中继的机器生成一对 3072 位密钥：

```bash
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 -out relay-private.pem
openssl pkey -in relay-private.pem -pubout -out relay-public.pem
```

私钥仅保留在服务端，并启动中继：

```bash
RELAY_RSA_PRIVATE_KEY_PATH=/secure/path/relay-private.pem \
LAN_DISCOVERY=true \
cargo run -p relay-server
```

将 `relay-public.pem` 通过可信方式放到每台客户端，再启动客户端：

```bash
CHICK_RELAY_URL=ws://192.168.1.20:8080/ws \
CHICK_SERVER_RSA_PUBLIC_KEY_PATH=/secure/path/relay-public.pem \
cargo run -p pet-client
```

一旦服务端设置 `RELAY_RSA_PRIVATE_KEY_PATH`，它会拒绝任何未完成加密握手的客户端；客户端设置 `CHICK_SERVER_RSA_PUBLIC_KEY_PATH` 后，也只接受加密帧。公钥文件就是此模式的服务端身份凭据，不能通过同一条未加密网络连接下载，必须随安装包、U 盘、受信任内网共享或其他可信渠道分发。

该方案保护业务内容和防篡改，但静态 RSA 密钥协商不具备 TLS 1.3 的前向保密能力；需要面向公网长期运行时，仍建议使用 `wss://`。`deploy/Caddyfile.no-domain` 和 `deploy/docker-compose.no-domain.yml` 继续可用于需要 TLS 的固定 IP 或纯内网场景。

如果宿主机的 80、443、8080 已被占用，可使用加固后的直连配置，仅映射外部 TCP 18080：

```bash
sh deploy/relay.sh up
```

首次启动自动准备配置和密钥，并验证健康状态。端口修改、升级、客户端连接和日常命令见 [服务端快速部署](deploy/QUICKSTART.md)。

该配置强制启用 RSA/AES 应用层加密，并从服务器本地的
`deploy/secrets/access-tokens.json` 加载随机访问令牌；任何缺失密钥、明文连接或
`dev:<name>` 登录都会被拒绝。密钥目录已从 Git 中排除，初始化脚本也不会覆盖已有密钥。

> `dev:<name>` 仅供开发，不可暴露到公网。直连方案已有随机令牌和配对权限检查，但没有完整登录站、令牌生命周期、速率限制、重放保护或媒体上传安全流程。

阿里云 CentOS 7.9 与 x64 NAS 的具体部署边界见 [deploy/centos7.md](deploy/centos7.md)。

## macOS DMG

当前公开稳定版为 v0.3.4，包含 Apple Silicon 与 Intel 架构的 macOS DMG；未做 Apple 公证。Windows x64 包也未做代码签名。源码构建与发布步骤见 [接手文档](offread.md) 和 `packaging/release.sh`。
