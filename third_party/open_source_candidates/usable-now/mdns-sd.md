# mdns-sd

- Repo: <https://github.com/keepsimple1/mdns-sd>
- License: Apache-2.0 OR MIT
- 技术栈：Rust crate

## 为什么适合当前项目

这个库本身就是你现在已经在用的 `mdns-sd`。继续借用它自己的上游示例和设计，是当前成本最低、契合度最高的选择。

## 最值得借用的部分

1. service announce / browse 的标准用法
2. 属性字段设计与解析约定
3. IPv4/IPv6 兼容思路
4. 守护线程和异常恢复策略

## 建议映射到本项目的位置

- `pet-client/src/lan_discovery.rs`
  从“只取一个结果”升级到“更稳的首选结果 + 超时回退”。
- `relay-server/src/main.rs`
  评估补充 IPv6 地址、验证广播字段、服务关闭清理。

## 迁移边界

- 可迁移：示例代码结构、字段命名、错误处理方式。
- 不建议重复造轮子：不要自己重写 mDNS 协议层。
