# Chick Link v0.3.4

双人共养桌宠正式发布包，包含聊天历史、消息气泡、同步拍拍、柔和提示音、
在线/离线状态提示，以及已部署服务端的配对持久化支持。

## 安装与连接

- macOS 11 及以上：打开 Universal DMG，把 Chick Link.app 拖到 Applications。
  同时包含 Apple Silicon 和 Intel 架构。不要从 DMG 中开启开机自启。
- Windows x64：推荐解压 ZIP 到固定的本地文件夹，双击 pet-client.exe。
  单独 EXE 为便携程序，不是安装向导；不建议从 NAS 直接运行。
- 连接入口：wss://pet.narendra.cn:18443/ws。两端不必处于同一局域网。
- 公共 GitHub 包不含任何私人令牌，也不提供公开注册；使用自己的服务端或由
  管理员提供各自独立的 connection 配置。切勿共享另一端身份。
- Windows 私用分发目录内已有 chicklink-connection.json 和 relay-public.pem，
  与 EXE 放在一起即可自动读取。升级时保留这两个文件。
- Mac 私人配置位置为 ~/Library/Application Support/ChickLink/connection.json，
  公钥放在同目录。应用升级不会覆盖聊天记录或该目录。
- 云端配置关闭 LAN 自动发现。退出旧测试客户端后再启动新版；开机自启需在
  安装到固定位置后重新确认。原有聊天历史和昵称不清空。

## 使用与限制

右键桌宠打开菜单，输入对方配对码即可请求共养；聊天历史保存在本机。
不要同时运行同一身份的新旧客户端，否则连接可能互相替换。

Mac 仅有临时签名，未取得 Developer ID 签名和 Apple 公证；Windows 未代码签名。
系统可能提示无法验证开发者。请核对官方下载和 SHA256，不要全局关闭系统安全保护。
跨平台构建不等于 Windows 真机验收，新包仍需在用户电脑上检查启动及聊天。

公网传输使用 TLS 和 RSA/AES；中继可解密消息，不是用户间端到端加密。
本版发布时旧云证书有效至 2026-12-10，自动续期尚未完成。（已过时；线上证书已续期至 2027-01-13 09:50:43 UTC，见 [接手文档](../offread.md)。）
2026-10-15 已替换服务端源码附件：原 ZIP 曾误含 LAN 测试私钥和令牌，
已撤下；当前 ZIP 经重新检查，不含已知私人令牌、RSA 私钥、DNS 密钥或 ACME 账号。
曾下载旧源码 ZIP 的用户应删除旧包，不要使用其中的 LAN 测试凭据。
