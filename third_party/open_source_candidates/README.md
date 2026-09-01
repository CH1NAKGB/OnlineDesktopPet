# Open Source Candidate Library

这个目录用于收集当前项目可借用的开源实现线索，目标不是盲目整仓搬运，而是把“能直接套到 Chick Link 的部分”先整理出来，方便后续按模块接入。

## 当前项目里最适合复用的点

1. 桌宠窗口增强：点击穿透、位置持久化、多显示器边界处理、开机自启。
2. 精灵与行为系统：从程序化小鸡切到精灵图、状态机、气泡与对话资源组织。
3. 中继服务增强：WebSocket 会话生命周期、房间/配对抽象、错误处理模式。
4. 局域网发现增强：更稳的广播与浏览处理、异常恢复、IPv4/IPv6 兼容策略。

## 候选项目分级

- `usable-now/`
  许可证和架构都相对适合，适合优先拆模块借用。
- `reference-only/`
  思路有价值，但许可证或技术栈不适合直接拷贝代码。

## 使用建议

1. 先读 `INTEGRATION_CHECKLIST.md`，决定我们下一步接哪块。
2. 再看对应候选项目说明，确认许可证和迁移边界。
3. 真要抄代码时，只抄最小必要文件，并保留上游 LICENSE/NOTICE。

## 已确认的上游来源

- QimoBar
  <https://github.com/Picrew/QimoBar>
- mdns-sd
  <https://github.com/keepsimple1/mdns-sd>
- axum chat example
  <https://github.com/tokio-rs/axum/blob/main/examples/chat/src/main.rs>
- rust-websocket-relay
  <https://github.com/AlexSua/rust-websocket-relay>
- openclaw-tamagotchi
  <https://github.com/katolikov/openclaw-tamagotchi>
- OpenPet
  <https://github.com/X-T-E-R/OpenPet>
