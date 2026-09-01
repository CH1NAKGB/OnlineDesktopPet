# Chick Link

一个面向 macOS 与 Windows 的轻量联机桌宠原型。桌宠主窗口使用原生窗口事件循环；中继服务采用 WebSocket，消息协议从第一天开始独立版本化。

## 当前结构

```
pet-client/       原生透明置顶窗口、拖拽、wgpu 渲染和帧率调度
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

客户端默认连到 `ws://127.0.0.1:8080/ws`，开发身份是 `dev:chick-user`。两个不同客户端使用不同的 `dev:<name>` 值即可互发事件；发布前必须替换为认证站签发并校验的短期 JWT。

## 帧率策略

- 活跃状态（拖拽、抚摸、收到消息）：请求连续重绘，渲染器将使用与显示器同步的 present mode。
- 空闲状态：50ms 唤醒一次，避免透明桌宠持续占用 GPU/CPU。
- 动画逻辑必须使用时间差，不应把角色速度绑定到 60 或 120 FPS。

桌宠目前以程序化原创小鸡绘制到真实透明窗口，主渲染链路不经过 WebView。下一步会把它替换为带 alpha 的精灵图，并增加逐像素点击穿透。

左键拖动会使用系统原生窗口移动；快速轻点小鸡会触发短暂的开心跳动。按 `Esc` 可退出测试窗口。

## 局域网联机与搜索

在一台 macOS、Windows 或 Linux 电脑上启动 `relay-server`，并设置 `LAN_DISCOVERY=true`。它会通过 mDNS 广播 `_chicklink._tcp.local.`；同一 Wi-Fi 或有线网段内的客户端会优先搜索这个中继，并连接 `ws://局域网地址:端口/ws`。搜索最多等待一秒，未发现时自动回退到配置的公网地址。

```bash
LAN_DISCOVERY=true cargo run -p relay-server
```

局域网模式需要系统防火墙允许 UDP 5353（mDNS）与 TCP 8080（开发期中继端口）。企业/校园 Wi-Fi 若开启了“客户端隔离”，设备彼此不可见，这是网络策略限制，客户端会自动回退到公网连接。

### Windows 真机验证

Windows 电脑安装 Rust 稳定版后，在同一仓库目录执行以下命令。中继可运行在 Mac 或 Windows 任一端，但只有运行中继的那台机器需要设置 `LAN_DISCOVERY=true`。

```powershell
cargo check --workspace
cargo run -p pet-client
```

若 Windows Defender 防火墙弹出提示，允许专用网络中的 `pet-client`。若中继也运行在 Windows 上，再额外允许 TCP 8080 和 UDP 5353。测试成功标准是：Windows 客户端在一秒内发现 `chick-relay`，并在中继日志中出现连接记录；关闭局域网中继后，客户端应回退到配置的公网 WebSocket 地址。

## 隐私边界

默认只同步在线、离开、离线。`SharedActivity` 只有在用户在设置页明确启用“分享当前应用”后才允许发送，且只允许应用显示名；不得上传窗口标题、文件路径、命令行参数、屏幕内容或完整进程列表。服务端后续还需要增加好友关系校验，不能向全部在线用户广播状态。

## 部署建议

阿里云实例：认证站、API、WebSocket 入口和 TLS 反向代理。NAS：图片对象存储、聊天媒体和备份。客户端只连固定域名，DDNS 仅作为 NAS 的上游地址，避免把动态地址暴露给用户。

中继容器的最小部署文件在 `deploy/`。在一台有 Docker 的 Linux 主机上，将 `deploy/.env.example` 复制为 `.env`，填写指向该主机的域名，并从仓库根目录执行：

```bash
docker compose --env-file deploy/.env -f deploy/docker-compose.yml up -d --build
```

域名必须先将 80/443 端口解析到主机，Caddy 才能自动申请 TLS 证书。NAS 采用 DDNS 时也可使用这套部署，但需要路由器端口转发和稳定的公网 IPv4/IPv6；没有稳定入站网络时，优先把中继放到阿里云。

> 安全提醒：当前 `dev:<name>` 是仅供本地调试的临时身份，不可暴露到公网。生产前需要认证站、JWT 签名校验、好友关系校验、上传 MIME/尺寸限制、速率限制和审计日志。

阿里云 CentOS 7.9 与 x64 NAS 的具体部署边界见 [deploy/centos7.md](deploy/centos7.md)。
