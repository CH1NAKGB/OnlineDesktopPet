# Chick Link v<version>

此文件用于新版本发布说明；复制为 `packaging/RELEASE-<version>.md` 后填写实际变更和验收结果。版本号以仓库根目录 `Cargo.toml` 为准。

## 更新

- 此处填写用户可感知的改动。
- 从旧版升级到带应用内更新功能的版本，需要先手动安装一次。

## 下载与校验

- macOS：`ChickLink-macos-universal-v<version>.dmg`；应用内更新使用同版本 `.zip`。
- Windows：`ChickLink-Setup-v<version>.exe`；便携版单独提供。
- 对照 `SHA256SUMS-v<version>.txt` 校验下载文件。

macOS 应用内更新通过 SHA-256 检查下载内容。当前构建使用临时签名，签名检查只能发现损坏，不能证明发布者身份。
