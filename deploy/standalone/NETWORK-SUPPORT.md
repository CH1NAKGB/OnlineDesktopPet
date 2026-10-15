# 阿里云出站连接诊断

> 历史故障记录。2026-10-15 服务器访问 Let's Encrypt 正式目录返回 HTTP 200，staging 和生产签发成功；当前证书状态见 `README.md`。以下工单正文无需再按原状提交。

此文件不含密钥，可作为工单描述。未创建或提交工单。

## 工单正文

北京地域 ECS 公网 IP 47.94.13.254，实例 i-2zec3mqkb9rb9nwqz2dh，
访问 Let's Encrypt ACME HTTPS 接口失败，影响独立应用的证书自动续期。
请协助检查该实例的安全组出方向、云防火墙及到目标的公网路由。
请勿重启实例、变更现有网站配置或清空防火墙规则。

2026-09-14 复测结果：

- 正式接口：acme-v02.api.letsencrypt.org:443，解析为 172.65.32.248。
- 测试接口：acme-staging-v02.api.letsencrypt.org:443，解析为 172.65.46.172。
- IPv4 TCP 连接阶段超时，time_connect=0，未进入 TLS 握手。
- 正式接口的解析与独立查询阿里公共 DNS 的结果一致。
- IPv6 返回 Network is unreachable，无默认 IPv6 路由。
- iptables OUTPUT 默认 ACCEPT，OUTPUT_direct 为空。
- 路由查询目标经 eth0 默认网关 172.20.127.253。
- 同实例访问 https://alidns.aliyuncs.com/ 返回 HTTP 400，证明该 HTTPS 目标可达。
- 现有桌宠 https://pet.narendra.cn:18443/healthz 公网访问正常。

请仅诊断 TCP 出站 443 到证书机构的问题；不是要求开放本站入站 443。
现有 80/443/8080 有其他业务占用，桌宠仅使用 18443。
以上 IP 是当时解析结果，不应作为永久 hosts 或防火墙地址列表。

## 最小复现命令

```sh
curl -4 -sS --connect-timeout 8 --max-time 15 -o /dev/null \
  -w 'http=%{http_code} ip=%{remote_ip} dns=%{time_namelookup} tcp=%{time_connect} tls=%{time_appconnect}\n' \
  https://acme-v02.api.letsencrypt.org/directory
```

仍未核验：阿里云控制面的安全组出站规则、云防火墙策略及运营商路径。
不能仅凭本机规则宣称故障一定在阿里云，也不应改全局 DNS 或禁用 TLS 校验。

## 恢复后的验收顺序

1. 两个目录接口成功返回 HTTP 200。
2. 运行 chicklink-cert-staging，验证 DNS 添加、签发、清理完整流程。
3. 核对 Lego v5 的证书输出路径、证书链和私钥匹配。
4. 验收生产签发、专用 TLS 切换、失败回滚和未到期时不重启。
5. 最后才安装并启用 chicklink-cert-renew.timer。

当前生产续期脚本及 timer 仍未部署；线上证书有效至 2026-12-10。
