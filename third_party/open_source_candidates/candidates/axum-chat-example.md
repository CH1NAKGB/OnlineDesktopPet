# axum chat example

已收录固定版本源码、页面和 MIT 许可证，见 [源码清单](../SOURCES.md)。独立运行入口见 [使用说明](../examples/README.md)。

- Repo: <https://github.com/tokio-rs/axum/blob/main/examples/chat/src/main.rs>
- License: MIT
- 技术栈：Rust + Axum

## 为什么适合当前项目

当前 `relay-server` 已经是 `axum + websocket`，所以官方示例最适合拿来对照整理连接生命周期、读写任务拆分和连接结束后的清理逻辑。

## 最值得借用的部分

1. `socket.split()` 后发送/接收任务拆分
2. 连接完成/退出时的统一清理
3. 共享状态组织方式
4. 事件广播与连接管理模式

## 建议映射到本项目的位置

- `relay-server/src/main.rs`
  把当前大函数继续拆成更小的连接阶段函数：
  - 握手
  - 登录
  - 配对
  - 消息转发
  - 断线清理

## 迁移边界

- 可迁移：任务拆分和生命周期结构。
- 不直接适配：示例里的公开聊天室语义，需要替换成当前的配对语义。
