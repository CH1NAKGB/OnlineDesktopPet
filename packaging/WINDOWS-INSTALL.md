# Chick Link Windows 一键安装

安装前从托盘或右键菜单退出 Chick Link。
双击 Setup.exe，使用默认目录并点击安装，完成后可直接启动。
默认目录仍为 `%LOCALAPPDATA%\Programs\ChickLink`，这是为了原地升级并保留启动项；无需管理员权限。
安装器创建桌面和开始菜单快捷方式，可在 Windows 应用列表中卸载。

NAS 的 `ChickLink-Setup-Private-v0.3.4.exe` 含 Windows 专属云端身份，
仅供这台 Windows 使用，不能公开分享或上传 GitHub。
首次安装会配置 `%APPDATA%\ChickLink\connection.json` 和公钥；若已有配置，
不会覆盖。已有旧 LAN 配置时请联系管理员迁移，不要删除聊天记录来修复连接。

GitHub 的 `ChickLink-Setup-v0.3.4.exe` 是公共版，不带任何个人令牌，
需要管理员另行提供连接配置或使用自己的中继。公共版不会自动获取私用云端身份。

升级和卸载均保留 `%APPDATA%\ChickLink` 下的聊天、昵称、配对码和连接配置。
不主动开启开机自启；原先已开启的启动项会迁移到新安装位置。
卸载只清理本安装路径对应的启动项，不删除其他程序或递归清空目录。

安装器未代码签名，Windows 可能显示未知发布者。仅使用可信来源并核对 SHA256，
不要全局关闭 Defender 或 SmartScreen。当前已验证打包和载荷，尚需 Windows 真机
验证首次安装、覆盖升级、运行中升级提示、卸载及重新安装。
