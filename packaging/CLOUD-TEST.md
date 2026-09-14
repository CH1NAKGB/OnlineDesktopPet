# Chick Link 云端测试（2026-09-12）

连接地址：`wss://pet.narendra.cn:18443/ws`。
本包关闭局域网自动发现，避免错误切回本地中继。原 LAN 测试包保持不变。

1. 退出旧版 Windows 小鸡，将本文件夹完整复制到本机，再运行 Start-ChickLink.cmd。
2. 保留 chicklink-connection.json 和 relay-public.pem；配置包含 Windows 专属令牌，不上传 GitHub、不分享给第三人。
3. Mac 使用自己的云端身份。任一端输入另一端配对码即可，云端配对与 LAN 配对相互独立。
4. 测试聊天、气泡、拍拍、换肤、提示音及上下线通知。两台电脑不再需要在同一局域网。

公网探测：PowerShell `Test-NetConnection pet.narendra.cn -Port 18443`。
健康检查：https://pet.narendra.cn:18443/healthz 应返回 ok；不代表已经配对。
新版服务端保存已确认的配对关系，中继重启后可恢复。由旧内存版本首次升级时可能需要重新配对一次；待确认的解除配对请求在重启后需重新发起。聊天历史仍保存在客户端本地。

证书有效期到 2026-12-10，当前为手动 DNS 申请，自动续期尚未配置；部署者必须在过期前续签。
