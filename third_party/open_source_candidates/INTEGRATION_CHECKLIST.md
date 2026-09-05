# 接入清单

以下是建议实施顺序，均未接入主程序。优先处理已有同技术栈源码的模块。

| 顺序 | 工作 | 目标位置 | 来源 | 完成标准 |
| --- | --- | --- | --- | --- |
| 1 | WebSocket 生命周期 | relay-server/src/main.rs | [Axum](candidates/axum-chat-example.md) | 任一读写任务退出后清理另一任务；重连不误删新连接；配对权限不变 |
| 2 | mDNS 事件与退出处理 | pet-client/src/lan_discovery.rs、relay-server/src/main.rs | [mdns-sd](candidates/mdns-sd.md) | 超时和错误路径释放 daemon；保留公网回退；验证注册、移除和多网卡场景 |
| 3 | 窗口位置与多屏恢复 | pet-client/src/main.rs、pet-client/src/config.rs | [QimoBar](reference-only/qimobar.md) | 先核验上游实现；移植后验证屏幕拔插、缩放及位置恢复 |
| 4 | 精灵动画与动作状态 | pet-client/src/renderer.rs、pet-client/src/windows_renderer.rs、assets/ | [桌宠参考](reference-only/openclaw-tamagotchi.md) | 先确认素材授权；macOS/Windows 动画一致；验证帧率和透明边缘 |

## 每次接入

- 从 SOURCES.md 的固定提交取最小必要部分，保留对应许可证。
- 在来源记录中注明修改文件与改动原因；美术资源单独核验授权。
- 使用项目协议替换上游演示协议，验证配对隔离、输入校验和加密边界。
- 独立示例的构建通过只代表编译可用，业务接入仍需要相应测试。

## 尚待调研

| 模块 | 当前资料缺口 |
| --- | --- |
| 原生渲染与像素级命中测试 | 尚未收录兼容 wgpu/winit 与 Windows GDI 的实现 |
| 本地聊天记录、配置和托盘 | 尚未收录可直接替换的模块 |
| 认证与加密 | 现有聊天室示例不提供本项目所需的安全协议 |
| 打包和部署 | 尚未收录适配当前发布流程的脚本 |
