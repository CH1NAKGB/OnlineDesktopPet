# Integration Checklist

## P1: 最值得先接

### 1. 窗口行为增强

- 目标文件：
  - `pet-client/src/main.rs`
  - `pet-client/src/config.rs`
  - `pet-client/src/windows_renderer.rs`
- 优先借用：
  - 位置持久化
  - 点击穿透开关
  - 多显示器越界修正
  - 开机自启入口
- 上游候选：
  - `usable-now/qimobar.md`

### 2. 精灵/状态机切换

- 目标文件：
  - `pet-client/src/renderer.rs`
  - `pet-client/src/windows_renderer.rs`
  - `assets/`
- 优先借用：
  - 精灵图资源格式
  - 动作状态机
  - 气泡文本与台词资源
  - 宠物配置文件
- 上游候选：
  - `usable-now/openclaw-tamagotchi.md`
  - `reference-only/openpet.md`

### 3. Relay 抽象增强

- 目标文件：
  - `relay-server/src/main.rs`
  - `shared-protocol/src/lib.rs`
- 优先借用：
  - 连接生命周期拆分
  - 等待中/已配对状态机
  - 房间或配对通道抽象
  - 更清晰的错误事件定义
- 上游候选：
  - `usable-now/axum-chat-example.md`
  - `usable-now/rust-websocket-relay.md`

### 4. mDNS 稳定性增强

- 目标文件：
  - `pet-client/src/lan_discovery.rs`
  - `relay-server/src/main.rs`
- 优先借用：
  - browse/announce 的异常恢复
  - IPv6 支持评估
  - 广播属性字段设计
- 上游候选：
  - `usable-now/mdns-sd.md`

## 许可证规则

- `MIT` / `Apache-2.0`：可优先考虑直接复用，接入时补第三方声明。
- `GPL-3.0`：只看架构和资源组织，不直接拷源码到当前仓库。
