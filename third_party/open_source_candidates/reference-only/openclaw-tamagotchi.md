# openclaw-tamagotchi

- Repo: <https://github.com/katolikov/openclaw-tamagotchi>
- 状态：跨技术栈参考；许可证和美术资源授权待逐项核验，未收录源码或素材。
- 技术栈：Python + Qt

## 为什么适合当前项目

技术栈不同，但这个项目已经把“精灵宠物真正像宠物一样活起来”所需的结构做全了：状态机、心情/饥饿等数值、对话气泡、宠物配置文件、资源目录化。

## 最值得借用的部分

1. idle / walk / sleep 这类动作状态机
2. 宠物行为参数配置文件
3. 台词/气泡按状态和心情组织
4. 精灵图与宠物目录结构

## 建议映射到本项目的位置

- `assets/`
  未来可以切成每只宠物一个目录，包含精灵图、配置和台词。
- `pet-client/src/renderer.rs`
  从程序化绘制切到精灵图渲染。
- `pet-client/src/main.rs`
  增加状态机 tick 和情绪/行为事件源。

## 迁移边界

- 可迁移：资源组织、状态机设计、数据结构。
- 不建议直接搬：Qt/Python 渲染实现本身。
