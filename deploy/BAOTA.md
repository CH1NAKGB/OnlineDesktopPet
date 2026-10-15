# pet.narendra.cn 宝塔部署

目标：阿里云 47.94.13.254，SSH 22，宝塔 Nginx。现有 80/443/8080 不占用、不停止。
公网入口 `wss://pet.narendra.cn:18443/ws`；中继后端 `127.0.0.1:18080`。
此文件是尚未实施的宝塔替代方案。当前生产环境使用独立 Caddy 与中继，状态和证书续期步骤见 [standalone/README.md](standalone/README.md)。

1. 在 DNS 管理处设置 pet.narendra.cn 的 A 记录为 47.94.13.254。域名不会隐藏端口，客户端必须带 :18443。
2. 在宝塔申请/导入覆盖该域名的有效证书。优先 DNS 验证，不需要占用 80/443；确认自动续期。
3. 将服务器源码包放到独立目录。首次部署复制 `deploy/baota.env.example` 为 `deploy/.env`；已有部署先备份配置和 secrets，不覆盖凭据。
4. 仅在隔离测试环境中使用配套版本的镜像与源码包；导入镜像后运行 `sh deploy/relay.sh start`，或用 `up` 编译。不要拿历史镜像运行当前账号源码。
5. 将 `pet.narendra.cn.nginx.conf` 的 server 块加入宝塔 Nginx http 配置。核对真实证书路径，先执行宝塔 Nginx 的 `-t` 检查，再重载。
6. 宝塔防火墙、系统防火墙及阿里云安全组放行 TCP 18443。18080 仅本机监听，不应公网开放。
7. 从外网检查 `https://pet.narendra.cn:18443/healthz`，再用两个不同令牌的客户端配对测试。

不要把此文件直接替换整个 nginx.conf，不要为申请证书停止现有网站。
如果宝塔使用 Apache/OpenResty 或自定义安装路径，先按实际环境适配，不能直接执行假定命令。

## 客户端免环境变量配置（开发中版本）

Windows 将 `chicklink-connection.json` 放在 pet-client.exe 旁；macOS 可放在
`~/Library/Application Support/ChickLink/connection.json`，相对公钥路径基于配置文件目录：

```json
{
  "relay_url": "wss://pet.narendra.cn:18443/ws",
  "access_token": "替换为本人的随机令牌",
  "discover_lan_relay": false,
  "server_rsa_public_key_path": "relay-public.pem"
}
```

`relay-public.pem` 通过可信渠道分发；完整 token 表和私钥只留在服务器。
配置包含个人令牌，限制文件权限，不上传 GitHub。不同用户不能共用同一份身份配置。
配置查找优先级：CHICK_CONFIG_PATH > 可执行文件旁的 chicklink-connection.json > 用户数据目录 connection.json。
单独的 CHICK_RELAY_URL/CHICK_ACCESS_TOKEN 等环境变量继续优先覆盖对应字段。
CHICK_DISCOVER_LAN=true/false 可覆盖搜索开关；云端默认关闭发现，避免误连测试中继。
双击或开机自启现在可以重新加载这份配置。移动程序/配置后确认相对公钥仍可找到。

本说明仅记录宝塔替代方案；阿里云主机已采用独立部署，不按本文操作。
