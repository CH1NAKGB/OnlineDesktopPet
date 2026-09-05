# Chick Link

一个面向 macOS 与 Windows 的轻量联机桌宠原型。桌宠主窗口使用原生窗口事件循环；中继服务采用 WebSocket，消息协议从第一天开始独立版本化。

当前版本：[v0.3.3](https://github.com/CH1NAKGB/OnlineDesktopPet/releases/tag/v0.3.3)。
新对话或新开发者先读 [offread.md 接手文档](offread.md)；部署直接看 [快速部署](deploy/QUICKSTART.md)，客户端验收看 [测试说明](packaging/TESTING.md)。

## 当前结构

```
pet-client/       原生桌宠、聊天 UI；macOS wgpu / Windows layered-window 渲染
relay-server/     WebSocket 在线与点对点消息中继
shared-protocol/  两端共享的序列化事件
assets/           原创小鸡概念资源与导出要求
```

## 开发启动

先安装稳定版 Rust：<https://rustup.rs/>。随后在仓库根目录运行：

```bash
cargo run -p relay-server
cargo run -p pet-client
```

客户端默认回退到 `ws://127.0.0.1:8080/ws`，开发身份是 `dev:chick-user`。两个客户端使用不同的 `dev:<name>` 并配对后即可互发事件。此模式仅用于本地/可信 LAN；公网测试使用强制加密和随机令牌的直连部署。认证站与短期 JWT 尚未实现。

如果改为 `wss://`，客户端默认信任系统证书。没有公网域名时，也可以通过 `CHICK_TLS_CA_CERT_PATH=/path/to/ca-or-cert.pem` 额外信任一份 PEM 证书或私有 CA 链。

## 帧率策略

- 活跃状态（拖拽、抚摸、收到消息）：按当前显示器刷新率调度，范围 30–240Hz；wgpu 使用 AutoVsync，Windows 桌宠走系统 layered-window 合成。
- 空闲状态：50ms 唤醒一次，约 20FPS，减少透明桌宠的 GPU/CPU 开销。
- 动画逻辑必须使用时间差，不应把角色速度绑定到 60 或 120 FPS。

桌宠目前以程序化原创小鸡绘制到透明窗口，主渲染链路不经过 WebView。两端共享呼吸、眨眼和抚摸渐变参数。60/120Hz 动画一致性单测不等于所有硬件都能实测跑满；精灵资源包和逐像素命中仍是后续方向。

左键拖动会使用系统原生窗口移动；快速轻点小鸡会触发短暂的开心跳动。按 `Esc` 可退出测试窗口。

## 共同宠物与交互

一只小鸡由两位主人共同操作。macOS 与 Windows 都可以右键小鸡显示原生交互菜单，提供轻拍、换肤、配对/聊天和退出等操作。Windows 桌宠不会显示在任务栏，并改用 `UpdateLayeredWindow` 与预乘逐像素 Alpha 渲染，不再依赖容易产生黑底或黑边的颜色键。

客户端每三秒测量一次 RTT，并在 macOS 菜单、Windows 托盘菜单和桌宠右键菜单中显示连接质量；短暂断线后会自动重连。每台端侧首次运行会生成并持久化 8 位配对码，连接中继后上报码值并查重。任意一端在“配对 / 聊天”中输入对方配对码后，服务端会原子绑定双方；发起端和接收端都会自动打开聊天、显示成功提示，无需另一端重复输入。只有已配对双方能互发消息、状态与共同宠物操作：

```bash
CHICK_ACCESS_TOKEN=dev:owner-a cargo run -p pet-client
```

另一台电脑使用 `CHICK_ACCESS_TOKEN=dev:owner-b`，并把 `CHICK_RELAY_URL` 指向同一个中继即可验证同步。当前中继的配对关系仍保存在内存中，服务重启后需要重新配对。“配对 / 聊天”会打开独立非透明 egui 窗口，提供本机配对码、历史气泡列表、快捷回复和多行发送区；历史以 JSONL 保存在用户应用数据目录。解除共同小鸡关系必须由发起方先确认，再由另一位主人确认，服务端收到双方确认前仍保持原配对关系。设置区还可选择开机自启：macOS 使用当前用户的 LaunchAgent，Windows 使用当前用户的注册表 Run 项，均无需管理员权限。收到在线文本时还会在桌宠上方显示 8 秒气泡，Windows 气泡和桌宠会随系统 DPI 动态缩放。表情、图片上传和云端离线历史仍属于后续媒体/数据库阶段。

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

Release 的 `ChickLink-macos-universal-v0.3.3.dmg` 包含 Apple Silicon 与 Intel 架构，使用临时签名，未做 Apple 公证。Windows x64 ZIP 也未做代码签名。源码构建与发布步骤见 [接手文档](offread.md) 和 `packaging/release.sh`。
