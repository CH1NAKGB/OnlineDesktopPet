# Chick Link 接手文档

> 面向新对话、新 AI 与协作开发者。先读本文件，再核对当前代码和 Git 状态。
> 更新日期：2026-10-15（Asia/Shanghai）。产品名与桌宠形象保持 Chick Link 小鸡。GitHub v0.3.4 是最新公开稳定版；账号与云同步源码已在公开的 main，尚无包含它们的正式安装包或生产部署。

## 2026-10-15 当前接手要点

- v0.4 路线图 P0-1 已由提交 `b74b1c7e1984c79240c28d51120a5c95220bae97` 固定。回归：`cargo fmt --all --check`、`cargo check --workspace --locked`、`cargo test --workspace --locked`、Windows GNU 目标 `cargo check`、`sh deploy/tests/relay-smoke.sh` 均通过。测试结果为客户端 42 通过/1 项真实音频设备测试忽略、中继 27 通过、协议 9 通过。双 RSA 私钥隔离中继测试让旧公钥与新公钥客户端分别握手并双向发送事件；无加密通道下注册、登录、恢复、Resume 在发送前被拒绝。实际启动测试：`CHICK_DISCOVER_LAN=true` 且无公钥以“LAN 自动发现需要预置中继 RSA 公钥”退出。账号连接缺 RSA 时聊天状态栏与桌宠气泡提示错误。沙盒内回环端口测试需允许本机监听；`block v0.1.6` future-incompat 警告仍在。部署 smoke 仅用假 Docker 验证命令连接，未验证真实容器运行。
- `0.3.3` 文档检索结果仅保留历史版本记录：`offread.md`、`ROADMAP-v0.4.md`、`deploy/centos7.md`、`deploy/QUICKSTART.md`、`deploy/standalone/README.md`、`packaging/LAN-TEST.md`、`packaging/TESTING.md`、`third_party/open_source_candidates/INTEGRATION_CHECKLIST.md`。当前工作流、脚本、Compose 版本均取 v0.3.4。
- P0-2 备份验收清单已写入 `deploy/standalone/BACKUP-RUNBOOK.md`。桌面副本仍是未加密同机临时副本；用户将自行加密，离线介质、密文解密恢复、业务登录/配对演练及新 ACME 凭据备份尚未完成。生产云库尚未启用，无生产云历史可恢复。此项不得标为已验收。

- 本轮修改开始前，`main` 与 `origin/main` 同步；提交 `74d3add` 已公开。此后以实时 `git status` 和 `git log` 为准，不能沿用下文旧的“未提交改动”判断。
- 2026-10-15 已修复证书续期：服务器 ACME 出站恢复、staging DNS-01 成功、生产新证书已切换；外部验证线上证书到期 **2027-01-13 09:50:43 UTC**，HTTPS 健康检查通过。`chicklink-cert-renew.timer` 已启用，每天 04:00 和 16:00 检查；当日手动重跑确认未到续期窗口时不重新签发或重启 TLS。旧的本机证书副本仍于 2026-12-10 到期，不要把它当作线上现状。
- 生产 RSA 私钥是旧客户端固定公钥的信任根。本轮源码新增双私钥过渡支持，生产尚未部署；轮换步骤见 `deploy/standalone/KEY-ROTATION.md`。旧钥丢失或泄露后，当前已装客户端不能自动安全地接受新钥。
- `deploy/standalone/CLOUD-ACCOUNTS.md` 的账号库备份步骤仍待执行。2026-10-15 用户明确允许 Workbench 经阿里云 OSS 中转后，已把生产 `relationships.json` 和新 TLS 证书/私钥下载到桌面受限目录；它们与服务器哈希一致。本机旧有 RSA 私钥和 token 映射也与服务器一致，且用桌面副本在回环端口完成了中继启动恢复检查。桌面目录仍是**未加密、同机临时副本**；新 Lego 生产 ACME 账号私钥、未来账号库与存储 key 不在其中，异地离线恢复尚未完成。NAS 目前暂停同步。
- 云历史、待发送队列和会话令牌在客户端本地以明文文件保存，只有文件权限隔离；服务端的加密存储不覆盖客户端文件，也不是端到端加密。
- 客户端已拒绝无 RSA 握手的用户发起云账号请求；本轮进一步阻止无加密连接发送任何云请求或旧版访问令牌，并要求 LAN 自动发现配置预置 RSA 公钥。LAN 测试包已有该公钥。
- 账号窗口现在明确提示服务器可读消息、本机账号内容仍以明文文件保存；本地文件加密尚未实现，不能把提示当成加密措施。
- 2026-10-15 完整 workspace 自动回归通过（客户端 42 通过、1 项真实音频设备测试忽略；中继 27 通过；协议 9 通过），Windows GNU 目标 `cargo check` 通过。Windows/macOS 真机安装及 128 MiB 生产资源限制测试仍未完成。`block v0.1.6` 仍有 future-incompat 警告。
- 公开注册与邀请码目前不在用户需求范围；账号库仍未在生产启用。账号上线前仍须完成备份恢复、双机验收和资源限制下的测试。

## 2026-10-12 开发记录（历史快照）

- 名称保持 **Chick Link**，桌宠保持原有小鸡绘制；本次未改变产品名或形象。
- 本地包含账号注册/登录、一次性恢复码、多设备账号、加密 SQLite 云历史、PNG/JPEG 表情、离线同步和待发送队列的开发改动。
- 账号开发线尚未替换生产服务；部署前先阅读 `deploy/standalone/CLOUD-ACCOUNTS.md`。

## 2026-09-19 开发记录（历史快照，以顶部 2026-10-15 状态为准）

### 进度结论

- 稳定版仍是 GitHub `v0.3.4`，现有 DMG、EXE、ZIP 和安装器没有被本轮未发布代码覆盖。
- 工作区正在开发账号与云同步线：用户名/密码登录、一次性恢复码、多设备共用同一账号、加密 SQLite 云历史、PNG/JPEG 表情库、离线同步和持久待发送队列。
- 账号开发线尚未发布、尚未启用生产账号库，也没有替换阿里云上的正式中继；不要把当前工作区直接发给普通用户。
- 正式线上专用中继仍使用 `wss://pet.narendra.cn:18443/ws`，只占用已授权的 TCP `18443`；不动宝塔的 `80`、`443`、`8080`，中继后端 `18080` 仍只应监听回环。
- 云端证书当时有效期记录到 2026-12-10；自动续期当时仍未完成。（已过时，见 2026-10-15 小节）

### 本轮已完成

- 账号存储：Argon2id 密码哈希、随机恢复码哈希、令牌哈希和 30 天会话；恢复密码会撤销旧设备会话。
- 多设备：同一账号可在多台设备登录；旧身份迁移或恢复后，旧连接会立即失效，原配对关系保留。
- 云消息：服务端加密保存正文和图片，伙伴权限、分页、去重、离线历史和表情库配额已实现。
- 客户端待发送队列：消息先落盘，收到服务端确认后才移除；断线/繁忙可自动重试，永久失败保留为待手动重试；队列支持重启恢复和二次确认清空。
- 本地历史命名空间同时绑定服务器和账号，切换服务器不会混用历史或游标。
- 安全边界：按来源 IP 限制认证请求，按账号限制业务请求；未认证请求不能消耗其他账号额度；服务端限制连接数、入站帧和出站内存预算。
- 旧客户端与云账号之间的混合聊天明确拒绝并提示升级，避免“发送成功但没有进入云历史”。

### 已验证与未验证

- 本次撤回品牌改动后需重新跑完整回归；客户端音频试听测试需要真实输出设备，默认忽略。
- 已覆盖：密码恢复撤销、多设备 fanout、旧身份迁移、按 IP 限流、账号隔离、加密历史/表情、同 client ID 重试去重、混合旧身份拒绝、待发送队列容量和重启恢复。
- 已完成 Windows GNU 目标的 `cargo check`；macOS 两架构和 Windows 新功能 release 包还没有完成真机安装验收。
- 尚未完成：生产账号库启用、真实阿里云灰度、128 MiB 资源限制下的压力测试、真实 Windows/macOS 双机账号验收、证书自动续期、正式新版本打包和 GitHub Release。
- 当前没有声称“账号功能已上线”或“新功能已经发布”。

### 协作与编辑纪律

- 每次接手先看 `git status --short` 和 `git diff`，不要重置、覆盖或清理未知改动。
- 手工修改使用 `apply_patch`；不要把密钥、令牌、恢复码、RSA 私钥、云存储 key、聊天内容写进 Git、GitHub 或 NAS。
- 不运行 NAS 上的可执行文件；NAS 仅用于同步源码/构建包，不能替代本地验证环境。
- 不重启、替换或修改宝塔、Nginx、MySQL 和其他业务；线上部署必须只操作 `/opt/chicklink` 专用文件和已授权端口。

### v0.3.4 正式使用安排

- 新增 Windows NSIS 一键安装器：packaging/windows/build-installer.sh public|private。
  公共 Setup.exe 不含令牌；私用 Setup 在 dist/private-v0.3.4，当前不执行 NAS 同步，不能上传 GitHub。
  默认当前用户安装，无需管理员权限，升级/卸载保留 APPDATA。见 packaging/WINDOWS-INSTALL.md。
  安装器编译及载荷检查不等于 Windows 真机安装/卸载验收。

- 两端源码与 NAS 的 pet-client 目录已逐文件核对一致，本次统一构建 macOS 双架构和 Windows x64。
- 公共 DMG/EXE/ZIP 不含身份令牌；NAS 私用 Windows 目录单独保留原有云端身份。
- Mac 云端配置安装到 ~/Library/Application Support/ChickLink/connection.json，公钥同目录；不清空历史和昵称。
- 本机旧中继 PID 75326 已核实工作目录并停止。日常使用阿里云 18443，不依赖本机中继。
- 20 项自动测试通过，音频硬件试听忽略 1 项。Windows 真机验收和开发者签名/公证仍不宣称完成。
- 云端证书自动续期当时仍待解决出站连接问题，当时证书 2026-12-10 到期；普通安全组空出方向规则默认允许。（已过时，见 2026-10-15 小节）
- 最新用户说明见 packaging/RELEASE-0.3.4.md。实际发布附件以 GitHub v0.3.4 及 SHA256SUMS 为准。
> 本文描述该时点的事实，不是永久指令；用户的新要求和实际代码优先。
> 不含真实令牌、私钥、SSH 凭据或聊天内容。

## 1. 三分钟接手

### 2026-09-12 阿里云部署（优先于后文预备方案）

- Workbench CLI 已完成用户本机认证；不要读取、打印或同步 ~/.workbench/config.json。
- 云端已运行：wss://pet.narendra.cn:18443/ws，HTTPS 健康、RSA/AES 登录和心跳公网测试通过。
- 采用 /opt/chicklink 独立中继 + Caddy 服务，无 Docker，不动现有 Nginx；详见 deploy/standalone/README.md。
- TCP 18443 公网永久开放已获用户明确确认并落实；18080 仅监听回环。
- 线上 Let's Encrypt 证书到期 **2027-01-13 09:50:43 UTC**；`chicklink-cert-renew.timer` 已启用，每日 04:00/16:00 检查。续期验收记录见 2026-10-15 当前接手要点及 `deploy/standalone/README.md`。
- 云端身份位于 deploy/secrets/aliyun；分端包 dist/cloud-20260912，禁止将完整密钥表、Mac 令牌或证书私钥同步到 NAS/GitHub。
- 原 LAN 测试目录保持不变。云端配对单独建立；新版新增 RELAY_STATE_PATH 持久化已确认配对，部署状态以 deploy/standalone/README.md 和云端 unit 为准。
- 2026-09-12 持久化已升级到云端，文件 /opt/chicklink/state/relationships.json；首个新配对成功后写入，已核对 unit、active 状态与公网健康。原业务 PID 未变。
- 持久化不包含待确认解除请求、昵称缓存、在线状态和配对码。旧内存中继首次升级仍可能需重配一次，此后保存过的关系可恢复；文件损坏拒绝启动而非清空。
- 自动续期已通过独立 DNS 验证子域及最小授权完成；Workbench ECS 权限不等于 DNS 权限，仍不授予整个主域的解析修改权。
- 本轮仅新增专用服务及端口规则，未重启现有网站、宝塔或数据库。

### 2026-09-10 最新网络与部署安排（覆盖旧 NAS 限制）

- 用户已明确要求同步新 NAS：`smb://192.168.8.1/PCLink/Aiproject`，挂载 `/Volumes/PCLink`。
- 源码目录 `Aiproject/OnlineDesktopPet`，Windows 测试包在其 `test-builds/20260910/windows`。
- Mac 当前 LAN IP 为 `192.168.8.239`，本轮加密中继监听 18081 并启用 mDNS。
- 阿里云 IP `47.94.13.254`，SSH 22；用户名与密钥登录信息待提供，用户说稍后部署。
- 目标域名 `pet.narendra.cn`；公共 DoH 当时返回 NXDOMAIN，需要配置 A 记录，接手应重查。
- 宝塔方案：公网 WSS 18443，后端 127.0.0.1:18080，保留 RSA/AES，不占 80/443/8080。
- 参阅 `deploy/BAOTA.md`、`deploy/pet.narendra.cn.nginx.conf`、`deploy/baota.env.example`。
- 新增连接 JSON：CHICK_CONFIG_PATH > exe 旁 chicklink-connection.json > 用户目录 connection.json；环境变量逐项覆盖。
- 相对公钥路径基于 JSON 目录；双击/自启可读取持久配置，尚无连接设置 UI。
- 新增 CHICK_DISCOVER_LAN=true/false；云端 JSON 默认关闭发现，LAN Windows 包开启发现。
- `packaging/prepare-lan.py` 生成测试产物，不打印令牌；`deploy/lan-test.local/` 的服务器私钥不外发。
- NAS 只分发 Windows 测试身份与公钥，不包含 Mac 身份或完整令牌表；这些测试凭据不上传 GitHub。
- 修复深色模式复选框悬停/按下文字变暗：通用控件保持主题文字色和中性底色，强调色用于边框；新增两种主题对比度回归测试。
- 小鸡上方的聊天气泡只显示消息正文，不带发送者昵称；聊天窗口和本地历史仍保留昵称。
- 气泡按文字实际排版尺寸加留白收紧并水平居中；Mac 不再强制撑满窗口，Windows 不再使用固定宽高，保留 DPI 缩放及两行省略逻辑。
- 新增伙伴在线监测：聊天页及右键/托盘伙伴标签显示在线、离线或未知；空闲气泡提示状态变化，不覆盖消息。自身断网显示未知而非误报对方离线。
- 服务端在配对、重连发送状态快照，当前连接关闭时通知伙伴离线；15 秒无入站数据释放连接，客户端 12 秒未收到 Pong 则重连。沿用已有加密 Presence 事件，不采集应用活动。
- 此功能需新版中继；本地测试中继重启会清空内存配对关系，需要重新配对。现有 v0.3.3 公开服务器包不含本次监测逻辑。
- 2026-09-11 已核对加密集成测试通过：配对在线快照、正常关闭离线通知、重连在线通知、15 秒无心跳离线通知；Windows release 与 Mac/中继构建通过。NAS 测试目录沿用 20260910/windows。
- 18 项自动测试通过；音频设备试听默认 ignored；Windows x64 release 构建通过，真机待用户测试。

后文涉及旧 NAS、缺少云端 IP 或仅环境变量配置的描述属于历史基线，以本段为准。

### v0.3.3 之后的开发中改动（尚未纳入上述 Release）

- 新增 `pet-client/src/sound.rs`：本地合成的拍拍短音（约 220ms）与收到消息双音（约 420ms）。
- 使用 rodio 0.20.1，关闭默认解码功能；有界工作队列、限频，播放结束释放音频输出。
- 配对页/聊天设置有“柔和提示音”开关，保存到用户数据目录 `sounds-enabled.txt`，默认开启。
- 网络事件通过 EventLoopProxy 即时唤醒 UI；远端拍拍触发本地动画、短音和状态提示，不回传产生回声。
- 收到拍拍时不覆盖正在显示的聊天气泡；连续拍拍会限制声音频率。
- 断线时不发送本地共享动作；超过 1 秒的排队拍拍会丢弃，避免重连后补播过期互动。
- 新增真实双客户端加密 WebSocket 集成测试：未配对拒绝、双向拍拍、换肤、文字和无发送端回声。
- 无协议版本变更，现有 v0.3.3 中继即可转发；两端更新客户端才能都获得新音效。
- 集成测试需要允许本机回环端口；音效试听测试默认 ignored，需要真实音频设备。

```sh
cargo test --workspace
cargo test -p pet-client preview_both_cues_on_audio_device -- --ignored
```

以下版本号、附件与 14 项测试记录特指已发布 v0.3.3，不能当作这些新改动已发布的证据。

这是一个原生 Rust 跨 macOS / Windows 联机桌宠，产品名 **Chick Link**。
两人共养一只小鸡，通过中继服务器配对、聊天、同步抚摸/换肤等互动。
已有可运行客户端和加密中继，不要从零重搭框架。

- 工作目录：`/Users/diki_links/Documents/Aiproject`，这个目录本身就是仓库根目录。
- GitHub：https://github.com/CH1NAKGB/OnlineDesktopPet
- 主分支：`main`；v0.3.3 代码提交：`9202f6e553af0525c097983523144fde946638c8`。
- 已公开 Release：https://github.com/CH1NAKGB/OnlineDesktopPet/releases/tag/v0.3.3
- Linux 服务端构建与健康检查记录：https://github.com/CH1NAKGB/OnlineDesktopPet/actions/runs/33964143902
- 最近完成：UI / R10 圆角、居中配对码、新字体、自然动效、部署辅助脚本、预编译镜像与各端发布。
- 实际阿里云服务器尚未部署，缺少连接信息。不要把 GitHub runner 测试说成阿里云部署完成。
- 当前用户要求是整理此接手文件，没有新指定的产品功能；下一步以用户新反馈为准。

接手先运行：

```sh
git status --short --branch
git log -8 --oneline
git remote -v
```

如果需要拉取更新，先 fetch / 检查差异；有本地改动时不要直接覆盖。
旧对话里的 PID、工具会话编号、临时目录不能当成当前仍存在的事实。

## 2. 用户目标与协作约束

- 轻量、低负载、小体积、跨 macOS / Windows，支持高 DPI 和多显示器。
- 用户希望 60Hz / 120Hz 流畅并配合垂直同步；这是一项目标，尚无全硬件跑满保证。
- 支持拖拽、抚摸、右键菜单、两人共养、昵称、聊天历史与小鸡上方气泡。
- 用户很在意界面协调：按钮真实圆角、边缘对齐、内容居中、系统深浅色适配。
- 配对码要易读有特色，倾向带斜杠零的等宽字体。
- 希望局域网搜索和联机，也支持公网中继。
- 用户拥有 Windows 测试机，会让 Windows 端 Codex 同时排错。动 Windows 代码前先确认当前协作状态，保留对方修复。
- 用户允许为项目安装必要环境，但具体权限/网络仍受当前工具环境约束。
- 用户明确要求保留已有 RSA 加密部分，不能为了测试成功静默删掉或关闭它。
- 不添加 AI / Codex 为 contributor，不添加 AI `Co-authored-by`。沿用用户自己的 Git 身份。
- 旧 NAS 路径：`smb://192.168.4.101/R5A/Aiproject/OnlineDesktopPet`。
- 用户曾暂停旧 NAS 同步；2026-09-10 已明确授权同步上述新 NAS，旧地址不再使用。
- 用户以前要求释放 NAS 文件占用但不要卸载挂载。不要主动卸载其 SMB 共享。
- 用户最近已授权整理并发布 GitHub，v0.3.3 已完成；未来发布按当前任务授权执行。

## 3. 仓库地图

| 路径 | 职责 |
| --- | --- |
| `Cargo.toml` / `Cargo.lock` | Rust workspace；三个 crate 版本现为 0.3.4；edition 2024 |
| `pet-client/src/main.rs` | winit 事件循环、菜单/托盘、拖拽、共享动作、网络线程和消息分派 |
| `pet-client/src/chat_window.rs` | 独立不透明 egui 聊天/配对窗口、主题、输入、设置、解除配对弹窗、字体 |
| `pet-client/src/renderer.rs` | 非 Windows 桌宠 wgpu 渲染与气泡文字 |
| `pet-client/src/chicken.wgsl` | macOS 原生绘制小鸡的着色器 |
| `pet-client/src/windows_renderer.rs` | Windows DIB / UpdateLayeredWindow 桌宠、气泡文字 |
| `pet-client/src/animation.rs` | 两端共用的时间驱动动画参数 |
| `pet-client/src/frame_pacer.rs` | 活跃与空闲刷新节奏 |
| `pet-client/src/config.rs` | 启动环境变量、配对码生成/持久化、昵称 |
| `pet-client/src/chat.rs` | 本地 JSONL 聊天历史 |
| `pet-client/src/startup.rs` | macOS LaunchAgent / Windows HKCU Run |
| `pet-client/src/lan_discovery.rs` | mDNS 搜索中继 |
| `pet-client/assets/fonts/` | 内嵌配对码字体、来源与许可证 |
| `shared-protocol/src/lib.rs` | 协议类型、限制、RSA/AES 实现、协议测试 |
| `relay-server/src/main.rs` | Axum WebSocket 中继、身份/配对/会话、鉴权和服务发现 |
| `relay-server/Dockerfile` | 同为 Debian bookworm 的构建/运行镜像 |
| `deploy/relay.sh` | 服务端初始化、启动、状态、日志、健康检查、停止 |
| `deploy/init-encrypted-direct.sh` | RSA-3072 / 随机令牌生成，拒绝覆盖 |
| `deploy/docker-compose.encrypted-direct.yml` | 当前优先使用的 18080 加密直连方案 |
| `deploy/QUICKSTART.md` | 部署与两端连接的完整操作说明 |
| `deploy/centos7.md` / `deployment-intake.md` | 系统边界、部署需要的信息 |
| `deploy/docker-compose.yml` / `Caddyfile` | 域名 WSS 方案，需可用的 80/443 |
| `deploy/docker-compose.no-domain.yml` | 其他无域名 TLS 参考方案 |
| `deploy/tests/` | 部署脚本 smoke test 与 fake Docker |
| `packaging/release.sh` | 在 macOS 上构建 Universal DMG、Windows ZIP、服务端源码 ZIP |
| `packaging/macos/Info.plist` | 应用版本、标识、LSUIElement |
| `packaging/TESTING.md` | 发布包测试步骤和已知边界 |
| `.github/workflows/relay-image.yml` | 手动触发的 Linux x64 Docker 构建、启动检查、镜像导出 |
| `third_party/open_source_candidates/` | 已整理的开源参考资料和部分上游源码 |
| `dist/` | 本机发布产物，Git 忽略，不代表源码已提交 |

## 4. 产品现状

### 已有功能

- 原生透明、置顶桌宠；Windows 小鸡窗口不进入任务栏，系统托盘提供操作入口。
- 左键原生窗口拖动；短点击触发抚摸，右键菜单提供聊天/配对、皮肤等操作。
- 两人共享小鸡动作与皮肤；桌宠由客户端原生绘制。
- 端侧生成 8 位配对码，上报中继查重。仅一端输入对方码，两端收到配对反馈。
- 自定义显示昵称，避免把内部 `owner-a` 等路由 ID 直接当用户名字。
- 独立不透明聊天窗口：文字发送、快捷回复、历史列表、Enter 发送、Shift+Enter 换行。
- 对方消息在小鸡上方显示约 8 秒气泡。
- 解除配对需要发起方本地确认，再由对方确认；等待期间保留关系。
- 解除配对请求在连接重建时恢复/通知的逻辑已存在，但仅限同一个未重启的服务器进程。
- 网络状态、心跳往返延迟显示。心跳约每 3 秒；显示的是中继 RTT，不是两电脑端到端 RTT。
- mDNS 中继发现、重连；连接失败通常等待约 2 秒再试。
- 系统深浅色主题、R10 控件圆角、小窗口滚动、DPI 缩放、开机自启选项。

### 尚未完成或不能当成完成

- 没有生产登录站、账号注册、JWT 验签与令牌生命周期管理。
- 图片/表情资源发送、媒体上传、图片查看器未形成完整产品链路。协议有类型不等于 UI 已实现。
- Unicode 表情可作为文字发送，但彩色 emoji 渲染/全部字形覆盖没有保证。
- 活动应用名称分享尚未完成：配置/协议保留隐私开关与类型，当前上线 presence 的 activity 为 None。
- 没有服务端数据库；配对/在线会话/昵称映射/解除请求均主要保存在内存。
- 服务器重启后要重新配对，没有云端离线历史队列。
- 没有客户端持久化的服务器配置 UI；云端地址、令牌、公钥路径主要来自启动环境变量。
- 没有完整的长会话历史分页、按配对对象隔离、加密落盘与保留周期管理。
- 未取得 Apple 公证或 Windows 代码签名。
- 未实际部署到用户阿里云/NAS；v0.3.3 Windows 最新 UI/动效仍需真机验收。

## 5. 动画与性能事实

- `animation.rs` 每帧计算一次 `[lift, stretch, eye, wing]`，供两端复用。
- 自然呼吸周期约 3.6 秒；抚摸动作周期约 0.85 秒；指数平滑控制进入/退出。
- 眨眼在 17 秒循环内错开安排，含一次双眨眼。
- macOS 将姿态写入 32 字节 uniform；修改 WGSL 结构时必须同步 buffer 大小/布局。
- Windows 在 CPU 上逐像素绘制，不能假定与 GPU 成本相同；高 DPI 下像素数量平方增长。
- `frame_pacer.rs` 活跃状态约持续 3 秒，按显示器刷新率调度，当前范围 30–240Hz。
- **空闲间隔 50ms，即约 20FPS**。不要对用户声称全时段恒定跑满桌面刷新率。
- wgpu 使用 AutoVsync；帧调度 + 垂直同步并不自动证明所有硬件下能达到最高帧率。
- 60/120Hz 动画单测只证明相同时间的动作参数一致，不能替代实际 FPS、功耗或掉帧测试。
- 优先保留原生拖拽和无忙等策略，性能优化应以真机测量为依据。

## 6. UI 与字体：最近反复反馈点

- 默认聊天窗口 460×720 逻辑尺寸，最小 360×560。
- 配对页可滚动；聊天头部分离网络状态；长昵称截断；聊天记录区为输入区预留空间。
- 配对页主要控件高 42，设置行控件高 36；统一 R10 与 1px 边框。
- `rounded_button()` 在每个按钮上显式设置圆角。
- `configure_style()` 先 `context.set_theme(...)`，再设置样式；系统 ThemeChanged 时重配。
- **坑：egui 深/浅色各存一套 Style。只改当前默认 Style 可能在首帧主题切换后丢失。**
- 去掉控件 hover/active 的 expansion，避免外扩造成边缘不齐。
- 配对码 TextEdit 显式 horizontal_align / vertical_align Center。
- 配对码 placeholder 为 `7KQ3M9XZ`。不要再写“例如”：该等宽字体链没有中文 fallback，会出现缺字。
- 配对码字库：`JetBrainsMonoSlashedNL-Medium.otf`，v2.304，约 203KiB，SIL OFL 1.1。
- 字体来源：https://github.com/sharpjs/JetBrainsMonoSlashed/releases/tag/v2.304
- 它是斜杠零、无连字版本，内嵌进可执行文件；两端不依赖用户安装字体。
- 配对码规则排除 `0、1、I、L、O`，不要为了展示斜杠零擅自修改协议规则。
- macOS 界面和气泡优先系统 PingFang.ttc，fallback STHeiti Medium.ttc。
- Windows 界面优先 msyh.ttc / simhei.ttf；气泡单独加载 simhei.ttf / msyh.ttf。
- 正文/按钮 14、聊天气泡列表文字 15、等宽输入 18、本人配对码显示 27 逻辑字号。
- 字体 LICENSE/AUTHORS 必须随客户端包分发。上游原始许可证有少量尾空格，不要误当密钥或功能差异。

## 7. 历史问题与不能随意撤掉的修复

| 问题 | 当前处理 / 接手注意 |
| --- | --- |
| 拖动慢于鼠标、抖动 | 使用 `window.drag_window()`，不要恢复为手动追鼠标 set_outer_position 循环 |
| macOS 小鸡倒置 | shader UV Y 翻转；修改坐标时注意窗口与 GPU 原点差异 |
| 黑底/黑边 | macOS 透明 surface 与 alpha；Windows 使用预乘 alpha 的 layered window，不能退回黑色色键 |
| Windows 右键聊天无响应/输入难点 | 菜单关闭后延迟原生激活与 focus；`process_deferred_actions()` 是必要路径 |
| macOS 拖动调整窗口尺寸卡顿 | pending resize / 约 40ms 延迟重配 surface；不要在每个 resize 事件无条件反复重建 |
| Windows 大透明窗口/尺寸异常 | 固定桌宠逻辑尺寸与 DIB 重建限制，防止错误显示尺寸形成巨大点击区域 |
| 配对后对方无反馈 | Paired 事件及双端 UI 更新已接入，注意 initiated_by_you 的旧消息默认值 |
| 重连清理误删新连接 | 服务端删除 session 前使用 same_channel 判断是否仍为旧连接 |
| 圆角改了但还是方 | 显式按钮 corner_radius + 主题初始化修复；最终仍需看真实运行界面 |
| 旧版本窗口影响验收 | 启动前核对运行程序路径；不要同时开多份本机相同身份客户端 |
| DMG 签名验证失败 | Documents/FileProvider 的 FinderInfo 等扩展属性可能干扰签名；在 /tmp 干净 staging 签名打包 |

修改 UI 后不要只凭编译通过就宣称视觉验收完成。之前界面自动化无法识别直接运行的
`pet-client`，因此很多交互/外观最终由用户肉眼确认。新环境若能截图则应补做。

## 8. 协议与加密边界

- PROTOCOL_VERSION = 2；配对码长度 8；文本最多 2000 chars；昵称最多 24 chars。
- 配对码合法范围：2–9 与 A–Z，但排除 I/L/O；端侧随机生成并保存在本地。
- 客户端持有可信渠道分发的服务器 RSA 公钥，RSA-OAEP/SHA-256 加密 32 字节会话密钥。
- 业务帧使用 AES-256-GCM，随机 12 字节 nonce；JSON 帧里的 nonce/ciphertext 为 Base64。
- **这是客户端到中继的应用层加密，不是两位用户之间的端到端加密。服务器能解密业务内容。**
- 静态 RSA 协商不具备前向保密；当前帧结构没有显式序列号/重放检测机制。
- 已有篡改密文拒绝测试，不代表协议已做独立安全审计。
- 服务端配置私钥后拒绝未加密业务；客户端配置固定公钥后仅接受加密服务端帧。
- 配置令牌 JSON 表后禁用 dev: fallback；随机 token 映射内部 user ID。
- 开发模式只有未配置令牌表时才允许 `dev:<valid-id>`，不要暴露到公网。
- REQUIRE_ENCRYPTION / REQUIRE_PROVISIONED_TOKENS 为 true 时缺少配置直接拒绝启动。
- 公钥必须通过可信渠道分发，不能从同一条未认证的连接自行下载并信任。
- 后续加固方向：有界发送队列、握手超时、速率限制、持久化身份/授权、重放保护、TLS/WSS。
- 当前发送通道含 unbounded channel，慢客户端/高流量的内存上限没有完整治理。
- 旧 Caddy Compose 只配置了网络/TLS，未挂载随机令牌表；不能误认为它拥有 encrypted-direct 方案的鉴权。公开使用前必须补齐。

## 9. 客户端配置与本地数据

| 环境变量 | 用途 / 默认 |
| --- | --- |
| CHICK_RELAY_URL | 默认 wss://pet.narendra.cn:18443/ws |
| CHICK_ACCESS_TOKEN | 默认空；旧版令牌按个人配置 |
| CHICK_DEVICE_ID | 默认 local-desktop |
| CHICK_PARTNER_ID | 旧开发辅助入口；正常使用配对码流程 |
| CHICK_PAIRING_CODE | 覆盖生成/本地读取的配对码，用于隔离测试 |
| CHICK_DISPLAY_NAME | 合法昵称优先于本地保存昵称 |
| CHICK_SERVER_RSA_PUBLIC_KEY_PATH | 应用层加密固定公钥 PEM 路径 |
| CHICK_TLS_CA_CERT_PATH | WSS 额外信任的证书/CA PEM 路径 |
| CHICK_OPEN_CHAT | 设置为 1 可启动时直接打开配对/聊天窗口 |

`discover_lan_relay` 当前默认 false，没有完整设置 UI。启用时每轮连接前最多搜索 1 秒，发现的 LAN 中继仍会优先于显式 URL；新版客户端要求同时预置中继 RSA 公钥，并拒绝向无 RSA 连接发送令牌或云请求。

本地数据：

- macOS：`~/Library/Application Support/ChickLink/`
- Windows：`%APPDATA%\ChickLink\`
- 文件：`pairing-code.txt`、`display-name.txt`、`chat-history.jsonl`。
- 历史是明文 JSONL，目前整文件读取，不按配对对象隔离。不要上传用户历史文件。
- 两个本机测试实例共用上述目录会共用配对码/昵称/历史；测试应显式设置不同 code/token。
- macOS 自启：`~/Library/LaunchAgents/com.chicklink.desktop-pet.plist`。
- Windows 自启：`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 的 `ChickLink` 值。
- 自启只记录当前可执行路径，**不保存终端里的云端环境变量**。
- 不应从 DMG 或临时构建目录启用自启；先放到稳定安装位置，否则以后路径失效。

## 10. 服务端与用户机器条件

- 阿里云：CentOS 7.9 64 位，低配置、约 3Mbps 带宽；80、443、8080 已被其他服务占用。
- NAS：黑群晖 x64，动态公网 IP / DDNS，高带宽；当前不要访问其局域网路径。
- 采用宿主机 TCP 18080 映射容器内部 8080，不要去停掉用户已有端口服务。
- 两端曾在同一局域网；用户后来外出，不能假定当前依然同网段。
- 部署前仍需：公网 IP、SSH 端口/用户名、可用密钥登录方式、sudo/Docker 权限、18080 安全组规则。
- 不要求用户在聊天里贴 SSH 私钥/密码。没有远程访问前不能声称已经部署。

## 11. 服务端快速操作

以下是旧版 Docker 部署记录。当前生产为独立 Caddy + 中继，不使用 Docker；新部署见 [deploy/standalone/README.md](deploy/standalone/README.md)。旧镜像须与当时的源码包配套，不能直接用于当前工作区。

完整说明在 [deploy/QUICKSTART.md](deploy/QUICKSTART.md)。先安装可用 Docker/Compose 与 OpenSSL。
低配置机推荐下载 Release 的服务端源码 ZIP 与预编译镜像 tar.gz。
解压源码 ZIP，进入有 Cargo.toml 的目录：

```sh
docker load -i /下载目录/ChickLink-relay-linux-x64-v0.3.3.tar.gz
sh deploy/relay.sh start
```

从源码编译启动：

```sh
sh deploy/relay.sh up
```

改端口：先 `sh deploy/relay.sh init`，编辑 `deploy/.env` 的 RELAY_PUBLIC_PORT，再 start/up。
脚本不自动安装环境、不自动开放防火墙。安全组与主机防火墙需要放行所选 TCP 端口。

```sh
sh deploy/relay.sh status
sh deploy/relay.sh check
sh deploy/relay.sh logs
sh deploy/relay.sh stop
```

脚本语义：

- init：创建缺失 .env；四个 secrets 文件齐全则保留；部分缺失时拒绝旋转身份。
- up：初始化 + Compose build/start + 有限次数 HTTP 健康探测。
- start：初始化 + `--no-build`；需要事先导入匹配版本镜像。
- check：容器内请求 /healthz；只证明服务存活，不证明公网连通或双端配对成功。
- 兼容 docker compose 与 docker-compose，前提是具体 Compose 版本支持配置中的字段。
- `relay.sh` 从 Cargo.toml 读取版本并传给 Compose；手动直接运行 Compose 时，核对 .env 的 `CHICKLINK_VERSION` 与源码版本一致。镜像 workflow 只接受明确指定且与源码版本匹配的现有 tag。

密钥文件生成在 `deploy/secrets/`：

- relay-private.pem：RSA 私钥，仅服务器。
- relay-public.pem：分发给两端的固定公钥。
- access-tokens.json：两个随机 token 到内部 owner-a/owner-b 的映射，仅服务器。
- client-tokens.env：用于按人分发令牌，不要把双方令牌一起发送。
- 目录 mode 0700；私钥与 token map mode 0644 用于非 root 容器 bind mount 读取；client-tokens.env 0600。
- `.gitignore` 排除 secrets/.env；`.dockerignore` 排除它们进入镜像上下文。
- 升级备份 .env 与 secrets，保留身份；不要覆盖密钥后让客户端继续信任旧公钥。
- `relay.sh` 主要管理默认 secrets 路径；若使用高级自定义挂载路径，需额外核对 Compose 配置。

容器：rust:1.98-slim-bookworm 构建，debian:bookworm-slim 运行，非 root chick 用户。
单 job、locked release 编译；只读根文件系统、tmpfs、drop capabilities、no-new-privileges、
192MB 运行内存限制、轮转日志、curl 健康检查。192MB 不是源码编译的内存上限。
CentOS 7 已停止维护；GitHub Ubuntu runner 成功不等于旧内核/Docker 环境必然兼容。

## 12. 开发、测试与发布

本机已有 Rust、macOS 编译环境及 Windows GNU 交叉工具链。接手时重新确认：

```sh
rustc --version
cargo --version
rustup target list --installed
command -v x86_64-w64-mingw32-gcc
```

最近使用的 targets：aarch64-apple-darwin、x86_64-apple-darwin、x86_64-pc-windows-gnu。
不要把 cargo check --target 当成 Windows 真机运行测试。

```sh
cargo fmt --all
cargo check --workspace
cargo test --workspace
cargo check -p pet-client --target x86_64-pc-windows-gnu
sh deploy/tests/relay-smoke.sh
```

部署 smoke test 在 /tmp 创建隔离密钥，Docker 是 stub，只测试脚本分派、重复初始化、
不完整凭据拒绝和失败码；真实容器验证由 GitHub workflow 负责。

本地开发示例：

```sh
# 仅开发/可信 LAN，不要直接公网开放 dev 身份
LAN_DISCOVERY=true cargo run -p relay-server
CHICK_ACCESS_TOKEN=dev:owner-a CHICK_PAIRING_CODE=7KQ3M9XZ CHICK_OPEN_CHAT=1 cargo run -p pet-client
```

另一端使用不同 token / 配对码，连接同一中继；LAN mDNS 服务 `_chicklink._tcp.local.`，
使用 UDP 5353，业务 TCP 默认 8080。公网 Compose 关闭 LAN 广播。

在 macOS 仓库根目录打包：

```sh
sh packaging/release.sh
```

脚本固定版本号 0.3.4，构建三个客户端 target，lipo 合并 Mac 双架构，/tmp staging
清除扩展属性后临时签名，生成 DMG、Windows ZIP、服务端源码 ZIP 及三包校验文件。
它不产生 Linux 镜像；如果后来加入镜像，必须重新生成包含四包的 SHA256SUMS。

Linux 镜像构建：

```sh
gh workflow run relay-image.yml --ref main -f release_tag=v0.3.4
gh run list --workflow relay-image.yml
# 将 RUN_ID 替换为本次运行编号
gh run watch RUN_ID --exit-status
gh run download RUN_ID --name relay-linux-x64-v0.3.4 --dir dist
```

workflow 将运行 `relay.sh up` / `check`，docker save + gzip，再上传 Actions artifact。
artifact 仅保留 7 天；正式 Release 的镜像是另行上传的长期附件。
运行成功后校验下载镜像与 relay-image.sha256，才上传 Release。

发新版本时同步修改：Cargo.toml/Cargo.lock、Info.plist 短版本/构建号、release.sh、
Compose image tag、workflow artifact/tag/file 名、部署/测试说明。不要覆写已发布旧版本当作新版本。
临时签名通过不等于 Apple Developer ID / 公证；Windows 包也未签名。

v0.3.3 验证记录：

- 14 项 Rust 工作区测试通过：客户端 4、中继 2、协议 8。
- macOS 两架构 release 编译、最终 DMG 内 codesign --verify --deep --strict、lipo 检查通过。
- Windows x64 release 编译成功，文件是 PE32+ GUI executable。
- Windows/服务端 ZIP 完整性通过；服务端包 38 个文件与发布源码比对一致，不含 secrets/.env。
- GitHub Linux x64 镜像构建、启动、HTTP 健康检查通过；未验证目标 CentOS 主机。
- 更早 v0.3.2 曾完成本地真实 macOS 客户端固定公钥+随机令牌的加密连接测试。
- Windows 最新 UI/动效的真机验收、长期稳定性、全 DPI/FPS 矩阵尚未完成。
- Rust 依赖 `block v0.1.6` 有 future-incompatibility warning，当前构建并未失败。

## 13. v0.3.3 发布附件

| 文件 | 约大小 | SHA-256 |
| --- | --- | --- |
| ChickLink-macos-universal-v0.3.3.dmg | 5.8MiB | 99bfc16ef841d2d8f2c5c3b58562fb5c881e61b074f05e43de2b16f4bc9aeb6a |
| ChickLink-windows-x64-v0.3.3.zip | 3.1MiB | b66041fc0e4dd719ce499bb319fa582fda06a86fbb6d28b499bcfb7e93304a0d |
| ChickLink-server-docker-source-v0.3.3.zip | 198KiB | 6beb813dcc87ec3b0958e29ad63d3de6a605b77881000ff063c95e11d95a5d36 |
| ChickLink-relay-linux-x64-v0.3.3.tar.gz | 32MiB | ea91d4b00aa2bae0dcf9a5eafb4e6c8685dacd6fdd45ac510ec401d725bd97c7 |

另有 SHA256SUMS-v0.3.3.txt。重新打包会改变压缩包校验值，不能用旧值代替新计算结果。
Release 是已公开版本，创建完成后核验了 isDraft=false 及上传附件摘要。
本接手文档及之后的说明纠正晚于 v0.3.3 发布，可能只在 main 上存在；
不能据此断言旧 Release ZIP 包含本文件。文档更新不应静默重打包并覆盖原附件/校验值。

## 14. 开源参考资料

`third_party/open_source_candidates/` 是另一项任务整理的资料，已随 v0.3.3 提交。
不要删除或误认为所有内容都是已接入模块。

- source/：固定提交的 Axum 聊天示例、mdns-sd query/register 示例及许可证。
- examples/：独立 Cargo workspace，可单独 check；没有并入主 workspace。
- candidates/：源码与当前项目的对应关系。
- reference-only/：QimoBar、openclaw-tamagotchi、rust-websocket-relay、OpenPet 的调研说明。
- SOURCES.md：上游提交、许可证、验证记录；SHA256SUMS：9 个原件的校验值。
- 不要把“参考说明存在”说成“源码已集成”。未核验的美术授权不能直接搬用。
- 用户最初提到星露谷小鸡只是方向，当前程序化美术不是授权星露谷资产。

```sh
shasum -a 256 -c third_party/open_source_candidates/SHA256SUMS
cargo check --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bins
```

## 15. 后续建议优先级（不是已获授权的新任务）

1. 根据用户实测继续修复 UI 输入、圆角、尺寸、DPI；尤其最新 Windows 包。
2. 持久化服务器地址/个人令牌/公钥配置，并与双击启动、开机自启联动。
3. 服务端 SQLite 等轻量持久化配对、昵称与待处理请求，明确升级/迁移策略。
4. 改善队列上限、重连/握手超时、速率限制、重放防护与安全测试。
5. 补充端到端双客户端集成测试、真实 60/120Hz 性能与资源占用测量。
6. 再逐步做图片、表情资源、媒体存储、登录站、可选活动分享。

不要把所有方向同时展开；用户的新需求优先。涉及协议变更要同时协调两端和服务端。

## 16. 接手工作习惯

- 先看 Git 状态和当前文件，不覆盖用户/Windows 端/其他任务的改动。
- 发布前检查 dist 对应的实际版本、源码提交与许可证；不要把旧二进制仅改文件名发布。
- 代码/脚本变更使用清晰的最小补丁；只因排版不要重写整个工程。
- 修改透明渲染、原生菜单或输入焦点必须安排真实窗口验证，不能只跑单测。
- 新 AI 不应承诺“100% 所有刷新率适配”“生产安全”“图片聊天完成”等未经验证事实。
- 不上传 `.env`、secrets、/tmp 测试密钥或用户聊天记录。
- 网络/GUI 工具如受权限限制，按当前环境请求授权；用户授权不等于工具已执行成功。
- 之前出现过自动审批因额度限制拒绝；后续用户说继续后重试成功。这是历史情况，不代表当前仍受限。
- 本接手文件后续也应随事实变化更新，尤其版本、部署状态、已知缺陷和验收结果。

## 2026-09-14 续期接手补充（历史快照，续期已于 2026-10-15 完成）

自动续期仍未验收，不要报告部署完成。用户已提供专用 chicklink-cert 凭据，
本机验证子域可读、主域被拒绝；凭据已安装服务器 root 私密目录。
Lego v5.4.1 已校验并安装，staging 测试因服务器访问 Let's Encrypt 测试/正式
HTTPS 接口超时而停止。AliDNS API 可达；当时未切换旧证书（到期 2026-12-10；已过时，见 2026-10-15 小节），
线上健康正常，其他业务未重启。正式续期 unit/timer 和部署脚本仅本地草稿，
未上传启用。详细路径、验证结果及下一步见 deploy/standalone/README.md。
先解决出站网络再验证 staging，不能将保存凭据等同于续期成功；绝不把 DNS
凭据、ACME 账号私钥同步到 NAS/GitHub。当前状态以本文顶部和 `deploy/standalone/README.md` 为准。

## 17. 给下一位 AI 的启动提示

可以将下面这段作为新对话首条消息：

> 请先阅读仓库根目录 offread.md，并核对 git status、最近提交及相关代码。
> 项目是 Chick Link 原生 Rust 双端桌宠，最新公开稳定版 v0.3.4。保留 RSA/AES、Windows
> layered-window 透明渲染、原生拖拽、延迟焦点恢复、显式 R10 和主题初始化修复。
> NAS 目前暂停同步，不覆盖协作改动，不添加 AI contributor。区分已实现与未验证，
> 根据我接下来描述的问题继续实施并验证。
