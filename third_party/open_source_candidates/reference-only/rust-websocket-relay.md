# rust-websocket-relay

- Repo: <https://github.com/AlexSua/rust-websocket-relay>
- License: 上游 README 标注 MIT；尚未收录固定版本及许可证全文。
- 技术栈：Rust + Actix，与本项目 Axum 不同。

## 为什么适合当前项目

它做的事情和 Chick Link 的中继目标更像：把两个对端通过一个轻量 relay 建立起来。虽然它的标识模型更简单，但“等待对端 -> 建立桥接 -> 断线恢复”的抽象很值得借。

## 最值得借用的部分

1. 成对连接的等待状态表达
2. 对端上线/断线事件建模
3. 简化 relay 热路径
4. 后续向 P2P 或房间抽象演进的思路

## 建议映射到本项目的位置

- `relay-server/src/main.rs`
  把“配对码注册”和“已绑定情侣关系”分成更清晰的两个层次：
  - pairing code registry
  - active pair session

## 迁移边界

- 可迁移：pair relay 抽象、状态转换思路。
- 不直接适配：它没有你当前的协议版本、消息类型校验和加密握手。
