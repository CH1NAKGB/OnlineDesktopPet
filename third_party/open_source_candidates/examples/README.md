# 独立示例入口

从 Chick Link 项目根目录执行。此清单将原始源码作为三个可执行程序构建，独立于主工作区。Axum 使用本项目锁文件中的 0.8.9，mDNS 使用 0.21.0；原始示例文件没有修改。

```sh
cargo check --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bins
cargo run --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bin axum-chat
```

Axum 启动后访问 http://127.0.0.1:3000，在两个浏览器标签输入不同用户名进行聊天。这是公开广播聊天室示例，接入 Chick Link 时需保留现有配对权限和加密协议。

在两个终端分别执行以下命令，观察注册和移除事件：

```sh
cargo run --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bin mdns-query -- _chick-example._udp
cargo run --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bin mdns-register -- _chick-example._udp chick-example chick-example-host --unregister
```

查询程序持续监听，使用 Ctrl+C 停止。注册程序广播演示服务，端口固定 3456，不提供实际业务服务。此服务类型仅供示例测试。

`source/` 中的 Cargo.toml 是上游原始资料，仅收录了部分仓库文件，不能用它直接构建；请使用本目录的清单。
