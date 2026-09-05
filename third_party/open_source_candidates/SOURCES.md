# 源码与版本记录

收录日期：2026-09-05。下列文件从 GitHub 原始地址按固定提交下载，未修改内容。

| 本地目录 | 上游固定提交 | 版本 | 许可证 | 收录范围 |
| --- | --- | --- | --- | --- |
| source/axum-chat | [tokio-rs/axum](https://github.com/tokio-rs/axum/tree/8762520da82cd99b78b35869069b36cfa305d4b9) | axum-v0.8.4 | MIT | examples/chat/src/main.rs、chat.html、Cargo.toml；axum/LICENSE 保存为本地 LICENSE |
| source/mdns-sd | [keepsimple1/mdns-sd](https://github.com/keepsimple1/mdns-sd/tree/5fcb34a1602a59d51a9d1696e01093834a599e4c) | v0.21.0 | Apache-2.0 OR MIT | examples/query.rs、examples/register.rs、Cargo.toml、LICENSE-MIT、LICENSE-APACHE |

Axum 示例选自 0.8 系列的固定版本；独立构建入口使用当前项目锁定的 Axum 0.8.9。mDNS 示例与项目 0.21 依赖系列一致。没有复制完整库实现，使用 Cargo 获取依赖。

`examples/Cargo.toml` 和本目录说明由本项目新增，不属于上游原件。原始许可证随源码保留，复制片段时也应携带相应声明。

## 完整性校验

从项目根目录执行：

```sh
shasum -a 256 -c third_party/open_source_candidates/SHA256SUMS
```

校验用于检测收录后的内容变化；上游定位依据是上表中的提交和文件路径。

## 验证记录

- 已检查源码内容、依赖声明和原始许可证。
- `cargo check --manifest-path third_party/open_source_candidates/examples/Cargo.toml --bins` 通过，三个示例均可编译；验证环境为当前 macOS 主机。
- 9 个上游文件的 SHA-256 校验全部通过；独立依赖版本保存在 examples/Cargo.lock。
- 未进行浏览器双端聊天或局域网多机实测，未接入 Chick Link 主程序。

## 其他调研来源

- [QimoBar](https://github.com/Picrew/QimoBar)：项目介绍确认 Tauri 2 + React + Rust；具体窗口实现和许可证全文待核验。
- [openclaw-tamagotchi](https://github.com/katolikov/openclaw-tamagotchi)：项目介绍列出动画、行为、台词和配置；未收录源码或美术资源。
- [rust-websocket-relay](https://github.com/AlexSua/rust-websocket-relay)：项目介绍为 Actix 双端中继，README 标注 MIT；未收录固定版本源码。
- [OpenPet](https://github.com/X-T-E-R/OpenPet)：保留原调研入口，本轮未核验，不作为可直接复制的依据。
