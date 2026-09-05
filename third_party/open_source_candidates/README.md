# Chick Link 开源复用资料库

当前收录 2 组上游源码、3 个独立示例入口，以及 4 个架构参考项目。整理日期：2026-09-05。

| 目录 / 文件 | 用途 | 当前状态 |
| --- | --- | --- |
| [source/](source/) | Axum 聊天示例、mDNS 查询与注册示例、原始许可证 | 已下载固定提交源码，未接入业务 |
| [examples/](examples/README.md) | 独立 Cargo 构建和运行入口 | 验证结果见 SOURCES.md |
| [candidates/](candidates/) | 两组源码与当前项目的对应关系 | 接入前仍需适配 |
| [reference-only/](reference-only/) | 窗口、精灵和其他 relay 方案 | 仅有调研说明，无源码 |
| [SOURCES.md](SOURCES.md) | 上游地址、提交、许可证、校验方法 | 来源记录 |
| [INTEGRATION_CHECKLIST.md](INTEGRATION_CHECKLIST.md) | 接入顺序与验收要求 | 后续实施清单 |

## 按功能查找

| 项目需求 | 可用资料 | 如何使用 |
| --- | --- | --- |
| WebSocket 收发任务与断线清理 | [Axum 示例](candidates/axum-chat-example.md) | 拆取生命周期结构，保留现有配对和加密逻辑 |
| 局域网发现、注册与移除 | [mdns-sd 示例](candidates/mdns-sd.md) | 对照现有 0.21 API 和事件处理 |
| 窗口位置、点击穿透、多屏 | [QimoBar](reference-only/qimobar.md) | 调研后移植到 winit，不能直接调用 Tauri API |
| 精灵、动作、台词配置 | [openclaw-tamagotchi](reference-only/openclaw-tamagotchi.md) | Python/Qt 架构参考，资源授权另查 |
| 两端等待与桥接状态 | [rust-websocket-relay](reference-only/rust-websocket-relay.md) | Actix 架构参考 |
| 宠物资源包与导入流程 | [OpenPet](reference-only/openpet.md) | 待核验授权，仅保留入口 |

先阅读 [示例使用说明](examples/README.md)，再按 [接入清单](INTEGRATION_CHECKLIST.md) 选择模块。原 usable-now 分类已撤掉，避免将说明文档误认为已适配代码。

本轮源码收录覆盖通信与发现。渲染、聊天记录、配置持久化、托盘、加密和部署尚未收录额外实现；不能据此认为全项目的复用调研已完成。
