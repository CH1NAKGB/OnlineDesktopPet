# 阿里云独立部署实况

## 2026-10-15 证书续期现状

服务器 ACME 出站访问已恢复。`chicklink-cert-staging.service` 完成 DNS-01 测试签发与清理；生产签发、证书链及私钥匹配校验、Caddy 配置校验和 TLS 切换均完成。公网 `https://pet.narendra.cn:18443/healthz` 返回 `ok`，外部 TLS 握手显示新证书到期 **2027-01-13 09:50:43 UTC**；中继和 TLS 服务均为 active。

`chicklink-cert-renew.service/timer` 已安装并启用，每日 04:00、16:00 检查。2026-10-15 手动复跑显示 Lego 跳过未到窗口的续期，部署脚本确认证书未变且未重启 TLS。首次计划执行为 2026-10-16 04:00 CST。下面的 2026-09-14 记录是历史故障过程，不再代表当前状态。

生产 `relationships.json`、新 TLS 证书/私钥已在用户明确允许 Workbench/OSS 中转后复制到桌面受限目录，与服务器哈希一致。RSA 私钥和 token 映射的本机旧有副本也与服务器一致。2026-10-15 已用桌面 RSA/令牌/配对副本在本机回环端口启动隔离中继并通过健康检查，且新 TLS 证书与私钥匹配。桌面目录仍未加密、仍在同一台电脑上；新 Lego 生产 ACME 账号私钥和未来账号库不在其中，异地离线恢复尚未完成。

2026-09-12：中继及 HTTPS 已运行，公网 TLS 和 RSA/AES 登录/心跳验证通过。
同日已部署配对持久化升级，生产 unit 的 RELAY_STATE_PATH 已核对，两项专用服务 active，公网健康检查通过。备份：/opt/chicklink/backups/state-upgrade-20260912185915。

- 域名：pet.narendra.cn；入口 wss://pet.narendra.cn:18443/ws。
- 主机：47.94.13.254，北京 CentOS 7.9，systemd 219。
- 安装目录：/opt/chicklink；运行账号 chicklink，无交互登录。
- chicklink-relay.service：127.0.0.1:18080，静态 musl Rust 二进制，RSA3072 + AES256-GCM，预置令牌认证。
- chicklink-tls.service：独立 Caddy 2.11.4，只监听 TCP 18443，禁用自动 HTTPS 和管理接口，使用专用证书。
- CPUQuota 各 25%（每秒 250ms CPU）；内存硬上限分别 128 MiB、160 MiB。
- 已开启两项专用服务的开机启动。没有安装 Docker、升级系统库或修改/重启现有 Nginx、宝塔、MySQL。
- firewalld public 区域仅新增 18443/tcp 的运行时及永久规则，没有 reload。用户已明确授权公网访问该端口。
- 原有 Nginx、数据库、面板 PID 与部署前一致。空闲时桌宠进程合计约 27 MiB RSS，数值会随负载变化。

## 状态及回滚

```sh
systemctl status chicklink-relay chicklink-tls
curl --fail https://pet.narendra.cn:18443/healthz
journalctl -u chicklink-relay -u chicklink-tls --since today
```

如需停用，仅停止并禁用这两项专用服务；不要操作现有 Nginx 或数据库。
撤销公网入口时仅删除 18443/tcp 的运行时与永久规则，不清空防火墙，不执行全局 runtime-to-permanent。
证书更新只替换 /opt/chicklink/secrets 中桌宠证书并重启 chicklink-tls；不重启现有网站。

## 私密文件与待办

### 2026-09-14 自动续期进度（未完成验收）

- chicklink-cert 凭据已通过本机只读权限测试：验证子域可读，主域访问返回 Forbidden.RAM。凭据未写入仓库。
- 公共 CNAME 为 `_acme-challenge.pet.narendra.cn` -> `_acme-challenge.acme-pet.narendra.cn`。
- 已安装 `/opt/chicklink/bin/lego` v5.4.1，官方归档校验通过；二进制 SHA256 为 `08958a0807295d2bad8eb9aa42bcc2c24cb31a3f327c4e4185c03d4d8a92c8ab`，已在 CentOS 7 执行帮助命令。
- `/opt/chicklink/renewal` 为 root 0700；两个 DNS 密钥文件为 0600。不读取或同步这些内容。
- 已安装 renew-certificate.sh 与 chicklink-cert-staging.service。测试签发停在 ACME 注册前的联网阶段，已停止；无成功签发，未切换线上证书。
- 服务器 AliDNS HTTPS 返回 400（可达），Let’s Encrypt staging/production directory 的 IPv4 HTTPS 均超时。需先解决服务器到这两个机构接口的出站连通性；不要改全局 DNS、防火墙或网站配置来试错。
- deploy-certificate.sh、chicklink-cert-renew.service/timer **仅为本地草稿，未上传、未启用、未端到端验证**。先成功完成 staging，再核对 Lego v5 实际证书输出路径和完整链格式，最后验收生产签发/回滚/到期前不重启及 timer。
- 本次线上两项桌宠服务 active，公网严格 TLS 健康检查 ok；原 Nginx/MySQL/宝塔 PID 均与部署前一致。证书有效期未变。
- 后续复查定位为 TCP 连接超时而非 TLS/证书错误：主机 OUTPUT ACCEPT、OUTPUT_direct 空，IPv6 无路由，公共 DNS 与服务器解析一致。云端安全组出站及上游路径尚未核验；详见 NETWORK-SUPPORT.md。续期脚本新增 ACME 连通性前置检查，失败时不进入 DNS 操作或证书部署。

- 服务器私密文件在 /opt/chicklink/secrets，root:chicklink 0640，目录 0750。
- 本机 deploy/secrets/aliyun 为独立云端身份；与 LAN 密钥、令牌不同。不要上传 GitHub 或整个同步 NAS。
- 当时本机旧副本位于 deploy/secrets/acme/config/live/pet.narendra.cn，**有效期至 2026-12-10 13:06:55 UTC**；这不是 2026-10-15 已续期的线上证书。
- **当时自动续期未配置**：首次采用 Certbot 手动 DNS 验证。（已过时；2026-10-15 的线上签发和 timer 验收见本文件顶部。）
- 新版通过 RELAY_STATE_PATH=/opt/chicklink/state/relationships.json 保存双向配对表，配对/解除成功前先原子落盘，重启后读取；文件损坏时拒绝启动，不悄悄清空。旧内存版首次升级可能需要重新配对一次。
- 持久化范围仅为已确认的配对关系；在线状态、昵称缓存、配对码和待确认解除请求不落盘。重启后待确认请求需重新发起，客户端重新上报昵称与配对码；聊天历史保留在各端。
- upgrade-state.sh 仅更新桌宠中继和专用 unit，保留旧版程序及 unit 并在健康失败时回滚；备份在 /opt/chicklink/backups。
- 云端客户端包在 dist/cloud-20260912，Mac/Windows 各自身份分开。只允许向用户 NAS 分发 Windows 子目录。
- 现有 GitHub v0.3.3 Release 不包含这些新功能；本次 Linux 部署基于 a2d7c12 源码静态构建。
- TLS 架构为客户端到中继加密，服务器持有解密能力，不是两位用户间的端到端加密。
