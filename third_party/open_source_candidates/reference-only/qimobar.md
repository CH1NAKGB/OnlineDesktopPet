# QimoBar

- Repo: <https://github.com/Picrew/QimoBar>
- 状态：跨技术栈参考；许可证全文与具体实现尚待固定版本核验。
- 技术栈：Tauri 2 + React + Rust

## 为什么适合当前项目

上游介绍使用 Tauri 2 + React + Rust。窗口位置、穿透、多屏和自启动是本项目希望调研的方向，具体实现需要逐项核验，Tauri API 不能直接用于 winit。

## 最值得借用的部分

1. 窗口位置持久化和恢复逻辑
2. 点击穿透开关的交互设计
3. 多显示器边界校正策略
4. 托盘/菜单项与桌宠显隐联动

## 建议映射到本项目的位置

- `pet-client/src/config.rs`
  增加窗口位置、缩放、显隐、点击穿透等持久化配置。
- `pet-client/src/main.rs`
  增加启动恢复、托盘/菜单切换，以及未来的点击穿透开关。
- `pet-client/src/windows_renderer.rs`
  Windows 侧补 `WS_EX_TRANSPARENT` 切换能力。

## 迁移边界

- 可迁移：功能拆法、配置字段、交互流程。
- 不建议直接搬：Tauri 窗口 API 调用本身。
