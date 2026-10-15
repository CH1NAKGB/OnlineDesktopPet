# 新版本发版检查

1. 只修改 `Cargo.toml` 的 `[workspace.package].version`，保持三段数字；同步更新 `Cargo.lock`。版本化的旧发行说明和文档可保留历史版本号。
2. 从 `RELEASE-TEMPLATE.md` 创建并填写 `RELEASE-<version>.md`，核对客户端 UI、安装器文件名、注册表版本、macOS Info.plist 与 Cargo 版本一致。
3. 完成 macOS 与 Windows 真机升级、回滚、持久化数据验收后运行 `packaging/release.sh` 和 Windows 安装器构建；检查公开包不含私钥、令牌及私用配置。
4. 核对发布附件含 macOS DMG、macOS ZIP、Windows 安装器、Windows 便携包、服务端源码包和本版本 `SHA256SUMS`；逐个校验摘要。
5. 只在新版本 Release 上传新附件。已发布的 v0.3.4 附件不用于自更新，也不补发同名 ZIP。
