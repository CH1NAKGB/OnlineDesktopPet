# Chick Link Windows 一键安装

双击 Setup.exe，首次安装使用默认目录。覆盖升级时安装器沿用原目录、跳过目录页，
尝试先让运行中的小鸡正常退出；超时后才强制结束。若程序文件仍被占用，安装会停止并提示手动退出，
不会跳过可执行文件。用更旧版本覆盖时会提示降级可能导致新版数据无法读取。
默认目录仍为 `%LOCALAPPDATA%\Programs\ChickLink`，这是为了原地升级并保留启动项；无需管理员权限。
安装器创建桌面和开始菜单快捷方式，可在 Windows 应用列表中卸载。

NAS 的 `ChickLink-Setup-Private-v0.3.4.exe` 含 Windows 专属云端身份，
仅供这台 Windows 使用，不能公开分享或上传 GitHub。
首次安装会配置 `%APPDATA%\ChickLink\connection.json` 和公钥。现有 `connection.json`
永不覆盖；现有公钥只有与 `packaging/superseded-keys/` 中明确列出的已废止公钥
逐字节相同时，才会先备份为 `relay-public.pem.superseded-<新版本>` 再更新。
其他自有公钥逐字节保留。轮换前须先完成双钥过渡，并将确实废止的旧公钥纳入白名单。
已有旧 LAN 配置时请联系管理员迁移，不要删除聊天记录来修复连接。

GitHub 的 `ChickLink-Setup-v0.3.4.exe` 是公共版，不带任何个人令牌，
需要管理员另行提供连接配置或使用自己的中继。公共版不会自动获取私用云端身份。

升级和卸载均保留 `%APPDATA%\ChickLink` 下的聊天、昵称、配对码和连接配置。
便携版升级只替换 `pet-client.exe`，保留同目录的 `chicklink-connection.json` 与
`relay-public.pem`；便携版不运行安装器的公钥白名单逻辑。
不主动开启开机自启；原先已开启的启动项会迁移到新安装位置。
卸载只清理本安装路径对应的启动项，不删除其他程序或递归清空目录。

安装器未代码签名，Windows 可能显示未知发布者。仅使用可信来源并核对 SHA256，
不要全局关闭 Defender 或 SmartScreen。当前已验证打包和载荷，尚需 Windows 真机
验证首次安装、运行中覆盖升级、降级确认、配置与公钥保留、卸载及重新安装。

macOS 应用内更新另有回滚保护：旧 `.app.bak` 会保留到新版本进入事件循环至少 10 秒，
且配置加载和网络线程启动成功后才清理；这不要求已经连上中继。若更早崩溃，
备份与 `update-pending.txt` 会留到下次启动再检查。更新日志位于本机
`~/Library/Application Support/ChickLink/update.log`。macOS 真机更新与回滚仍未验收。
