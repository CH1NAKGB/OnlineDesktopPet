# 账号与加密 SQLite 部署说明

核对日期：2026-09-19。本文依据当前工作区实现编写，不代表已发布版本具备这些功能。

**生产尚未启用账号和云同步。本次仅编写说明，没有执行部署、生成生产密钥、迁移生产身份或重启服务。** 仓库中的独立部署 unit 尚未设置 `RELAY_CLOUD_DB` 和 `RELAY_CLOUD_KEY_PATH`。本文未连接生产主机复查实时配置，不把本地代码状态当作线上验收结果。

## 1. 启用条件与文件权限

当前启用入口为 `relay-server/src/cloud_service.rs::open_store`：只有设置了 `RELAY_CLOUD_DB` 才打开账号库。不设置时账号功能关闭；设置为空字符串不是安全的关闭方式。

| 配置 | 当前要求 |
| --- | --- |
| `RELAY_CLOUD_DB` | SQLite 文件路径。建议绝对路径、独立的本机磁盘目录；父目录须事先创建，服务账号须能创建数据库、`-wal` 和 `-shm` 文件。不要放在共享网络文件系统上。 |
| `RELAY_CLOUD_KEY_PATH` | 文件内容必须恰好为 **32 个随机原始字节**，不是 64 字符十六进制、Base64、口令或带换行文本。启用账号时必填。 |
| `RELAY_RSA_PRIVATE_KEY_PATH` | 必须可读、可解析为 **PKCS#8 PEM RSA 私钥，至少 2048 位**。账号连接强制经过 RSA 握手；测试示例使用 3072 位。 |
| `RELAY_RSA_PREVIOUS_PRIVATE_KEY_PATH` | 可选的过渡期旧私钥，仅在有主私钥时使用。轮换步骤见 `KEY-ROTATION.md`；旧钥泄露时不能继续启用。 |
| `RELAY_REQUIRE_ENCRYPTION=true` | 独立部署保留此设置，缺少 RSA 私钥时拒绝启动。 |
| `RELAY_ACCESS_TOKENS_PATH` | 原预置 token 到旧 user ID 的 JSON 映射。若配置，文件必须非空；token 为 32..512 字节，user ID 最长 64 字节且只含 ASCII 字母、数字、`-`、`_`、`.`。迁移时保留原映射。 |
| `RELAY_REQUIRE_PROVISIONED_TOKENS=true` | 独立部署保留此设置。它要求加载预置 token 文件，**不是账号注册邀请制**；新账号注册并不要求提供旧 token。 |
| `RELAY_STATE_PATH` | 配对关系 JSON，与账号 SQLite 分开保存。迁移保留原路径和内容；未设置时配对关系不持久化。 |
| `BIND_ADDR` | 测试使用独立回环端口；独立部署现有值为 `127.0.0.1:18080`，由专用 TLS 代理转发。 |
| `LAN_DISCOVERY=false` | 独立部署和隔离测试均关闭 LAN 广播。 |

现有独立部署的 RSA、token、配对文件路径分别为 `/opt/chicklink/secrets/relay-private.pem`、`/opt/chicklink/secrets/access-tokens.json`、`/opt/chicklink/state/relationships.json`。以后启用账号时，可另行配置 `/opt/chicklink/state/cloud.sqlite3` 和 `/opt/chicklink/secrets/cloud-storage.key`；这些是路径建议，本文没有创建文件或修改 unit。

权限要求：

- 运行账号为 `chicklink`；数据库父目录由它拥有，权限 `0700`，数据库及 WAL/SHM 不向其他用户开放。进程保持 `UMask=0077`。
- 私钥、云存储 key 和 token 文件可以使用 `root:chicklink 0640`，所在 secrets 目录 `root:chicklink 0750`，且该组只能包含受信任服务账号。若运行账号直接拥有文件，则用 `0600`、目录 `0700`。
- 程序检查文件内容，不自动修正权限。存储 key 只通过路径加载，不放进环境变量的值、命令行参数、日志、聊天记录或仓库。不要打印请求或认证结果的 `Debug` 内容。
- RSA 私钥、云存储 key、TLS 私钥是三种不同密钥，不能互相代替。只向客户端分发可信渠道取得的 RSA **公钥**，不能分发任何私钥或云存储 key。
- 已有数据库启动时校验加密标记；错误 key、缺失标记或损坏标记会拒绝启动。不得删库、删标记或重新生成 key 来绕过错误。当前没有原地存储密钥轮换接口。

## 2. 只在独立测试目录试用

以下均为**待人工执行的测试示例**，不是本次已执行操作。使用独立 OS 测试用户，或为每个测试客户端进程提供独立 HOME 和对应平台的数据目录，避免覆盖真实客户端的设备身份、登录态与历史；仅更换连接配置文件不足以隔离这些数据。需要已构建并核验来源的新版 relay、OpenSSL，以及做备份演练时使用的 SQLite CLI。

每次创建全新的临时目录，绝不复用生产数据库、key、RSA 私钥或原 token 文件。`/tmp` 仅用于可丢弃测试，不可用作生产持久化目录。生成过程只写文件，不输出秘密值；不要用 `set -x` 执行。

```sh
set +x
set -eu
umask 077
CLOUD_TEST_DIR=$(mktemp -d /tmp/chicklink-cloud-test.XXXXXX)
mkdir "$CLOUD_TEST_DIR/state" "$CLOUD_TEST_DIR/secrets"
chmod 700 "$CLOUD_TEST_DIR" "$CLOUD_TEST_DIR/state" "$CLOUD_TEST_DIR/secrets"

openssl rand -out "$CLOUD_TEST_DIR/secrets/cloud-storage.key" 32
test "$(wc -c < "$CLOUD_TEST_DIR/secrets/cloud-storage.key")" -eq 32
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:3072 \
  -out "$CLOUD_TEST_DIR/secrets/relay-private.pem"
openssl pkey -in "$CLOUD_TEST_DIR/secrets/relay-private.pem" -pubout \
  -out "$CLOUD_TEST_DIR/secrets/relay-public.pem"

cloud_test_token_a=$(openssl rand -hex 32)
cloud_test_token_b=$(openssl rand -hex 32)
printf '{"%s":"test-owner-a","%s":"test-owner-b"}\n' \
  "$cloud_test_token_a" "$cloud_test_token_b" \
  > "$CLOUD_TEST_DIR/secrets/access-tokens.json"
unset cloud_test_token_a cloud_test_token_b
chmod 600 "$CLOUD_TEST_DIR/secrets/"*
```

把下列可执行文件占位路径替换为已验证的测试 binary，并确认 `127.0.0.1:28080` 未占用。只在前台启动隔离实例，不运行现有 `install.sh`、`upgrade-state.sh` 或生产 `systemctl` 操作。

```sh
CLOUD_TEST_RELAY=/absolute/path/to/verified/relay-server
test -x "$CLOUD_TEST_RELAY"
env BIND_ADDR=127.0.0.1:28080 LAN_DISCOVERY=false \
  RELAY_REQUIRE_ENCRYPTION=true \
  RELAY_REQUIRE_PROVISIONED_TOKENS=true \
  RELAY_RSA_PRIVATE_KEY_PATH="$CLOUD_TEST_DIR/secrets/relay-private.pem" \
  RELAY_ACCESS_TOKENS_PATH="$CLOUD_TEST_DIR/secrets/access-tokens.json" \
  RELAY_STATE_PATH="$CLOUD_TEST_DIR/state/relationships.json" \
  RELAY_CLOUD_DB="$CLOUD_TEST_DIR/state/cloud.sqlite3" \
  RELAY_CLOUD_KEY_PATH="$CLOUD_TEST_DIR/secrets/cloud-storage.key" \
  "$CLOUD_TEST_RELAY"
```

测试客户端使用 `CHICK_RELAY_URL=ws://127.0.0.1:28080/ws`，`CHICK_SERVER_RSA_PUBLIC_KEY_PATH` 指向本次测试的 `relay-public.pem`。`ws://` 仅用于本机回环测试，公钥仍须匹配；远程部署保留校验证书的 `wss://`。不要将测试凭据填入现有生产客户端配置。

云账号设备身份由 `cloud_client::device_id()` 随机生成，持久化到云数据目录的 `device-id` 文件并在进程内缓存；`CHICK_DEVICE_ID` 仅用于 legacy 配置，不能指定云账号设备身份。模拟不同设备须使用独立 OS 用户，或在启动各自客户端进程前隔离数据目录；同一模拟设备重启时保留自己的目录，不要复制其他设备的 `device-id`。macOS 云数据位于 HOME 下的 `Library/Application Support/ChickLink/cloud`；Linux 优先使用 `XDG_DATA_HOME/ChickLink/cloud`，未设置才使用 HOME 下的 `.local/share/ChickLink/cloud`，因此还须隔离或清除继承的 `XDG_DATA_HOME`，并隔离 legacy 配置使用的 `XDG_CONFIG_HOME`；Windows 使用 `APPDATA/ChickLink/cloud`，仅改变 HOME 无效，优先使用独立 OS 测试用户。

验收至少覆盖：

1. 使用独立账号注册、登录和重启恢复；保存注册时返回的恢复码。密码长度为 10..128 **字节**，不是字符数。
2. 两个测试旧身份先配对，再分别迁移，确认 user ID 和配对关系保留，旧 token 拒绝再次登录。
3. 配对双方测试文本、PNG/JPEG、离线历史和多设备同步；相同 client ID 重试不重复入库，改变内容或收件人会被拒绝，第三个账号看不到双方历史。
4. 恢复密码后所有旧会话失效，旧密码和旧恢复码不能再用，新恢复码只在此次恢复结果中返回。退出登录也应撤销对应 token；普通会话固定 30 天有效，Resume 不延长有效期。
5. 在备份副本上用另一份随机 key 测试拒绝启动，再用原 key 验证恢复；不覆盖原 key，不在正在运行的数据库上做破坏性测试。
6. 达到配额时拒绝新增，不删除旧历史；完成下面的一致性备份与恢复演练。`/healthz` 返回 `ok` 仅说明服务响应，不证明上述行为已验收。

## 3. 一致性备份与恢复

**不能只 `cp`、`rsync` 或上传正在运行的 `cloud.sqlite3` 主文件。** 已提交数据可能仍在 `cloud.sqlite3-wal` 中；运行中分别复制 DB/WAL/SHM 也不保证它们属于同一时点。使用 SQLite Online Backup API 或 SQLite CLI 的 `.backup` 制作一致快照，检查命令成功后再归档。备份机制参见 [SQLite Online Backup API](https://www.sqlite.org/backup.html) 和 [WAL 文件说明](https://www.sqlite.org/wal.html)。

以下只演示对上节**测试库**做在线备份。在另一终端使用同一个测试目录路径；`CLOUD_TEST_DIR` 必须指向实际存在的测试目录，不能指向生产路径。备份目标必须全新，目录不带单引号。

```sh
set -eu
umask 077
test -n "${CLOUD_TEST_DIR:?Set the existing isolated test directory}"
test -f "$CLOUD_TEST_DIR/state/cloud.sqlite3"
CLOUD_TEST_BACKUP_DIR=$(mktemp -d /tmp/chicklink-cloud-backup.XXXXXX)
sqlite3 -bail "$CLOUD_TEST_DIR/state/cloud.sqlite3" <<EOF
.timeout 5000
.backup '$CLOUD_TEST_BACKUP_DIR/cloud.sqlite3'
EOF
chmod 600 "$CLOUD_TEST_BACKUP_DIR/cloud.sqlite3"
sqlite3 -bail "$CLOUD_TEST_BACKUP_DIR/cloud.sqlite3" 'PRAGMA integrity_check;'
```

`integrity_check` 应只返回 `ok`，但它只校验 SQLite 结构，不能证明 AES-GCM 数据可解密。备份失败或超时就标记失败，不把半成品当作有效备份；使用 [SQLite CLI 文档](https://www.sqlite.org/cli.html) 核对本机工具支持的选项。

备份与恢复要求：

- 数据库备份和**对应原始 32 字节存储 key 分别保存**在访问控制独立的备份位置，使用非秘密的备份编号关联。不能仅把两者放进同一备份目录或同一无额外保护的压缩包。测试 key 也不可误作生产 key；生产备份不得放 `/tmp`。
- key 遗失后，账号密码、恢复码和 RSA 私钥均不能恢复云消息/表情明文；数据库副本也不能重建 key。保留对应 binary 版本、配置路径说明、RSA 密钥恢复资料和 token 映射的受控备份，不在普通备份清单中写秘密值。
- `RELAY_STATE_PATH` 的配对 JSON 不包含在 SQLite `.backup` 内。需要同时恢复两种状态时，应在维护窗口停止这个 relay 的写入，再形成同一恢复批次；单独在线备份 SQLite 不构成跨文件事务快照。
- 恢复先在全新隔离目录和独立端口进行，用备份 DB、匹配 key 和配对文件验证启动、登录、历史、表情及迁移声明。不要覆盖运行中的数据库，也不要混入目标目录残留的其他版本 WAL/SHM。
- 恢复旧快照会回退账号、恢复码、token 撤销状态、已迁移身份声明及历史游标。此前被撤销的 token 可能重新有效，快照之后迁移的身份可能再次接受旧 token，消息序号与客户端游标也可能不再匹配。必须在开放访问前处理这些状态，不能只凭健康检查通过就上线。
- 实际切换数据库、停写、备份密钥和恢复生产均需另行安排；本说明未执行这些操作。

## 4. 原 token 迁移与回滚边界

先升级配对双方及各自所有在用设备，确认新版 relay 和客户端协议兼容，再安排双方各自迁移。注册请求携带自己原有的 `legacy_token`，服务端通过原 token 映射验证身份，将原 user ID 原子地绑定至新账号，保持既有配对关系。迁移声明唯一，不能由另一个账号重复领取。

**某个 user ID 一旦成功迁移，该身份的旧 token 登录立即失效，现有旧身份连接也会被断开。** 未升级的同身份设备必须改用新账号登录。迁移 A 不会直接撤销尚未迁移的 B 的 token，但当前协议层明确拒绝两个方向的混合消息：云账号向尚未迁移的旧身份发送云消息会被拒绝，旧身份向已迁移云账号发送旧协议消息也会被拒绝；云账号自身使用旧协议发送消息同样被拒绝。双方须完成升级并登录云账号后再聊天，不能将单边升级作为兼容方案。客户端旧本地聊天记录也没有由服务器迁移接口自动导入云库的保证。

回滚不能简单等同于“换回旧 binary、取消 `RELAY_CLOUD_DB`”：

- 已迁移声明保存在 SQLite 内。禁用云库、换回不检查声明的旧版本、恢复迁移前 DB 或换用空 DB，都可能重新放行原 token。**已 claim 的旧 token 不应因回滚而重新启用。**
- 应保留原库、key 和迁移声明记录，并在任何恢复旧认证逻辑前，使外部 token 映射里的已迁移身份凭据继续不可用；否则保持相关入口关闭。不能删除声明记录来让旧客户端继续登录。
- 未提供安全自动降级、批量密钥轮换或迁移声明重放工具。现有 `upgrade-state.sh` 面向配对持久化升级，其旧版 binary/unit 回滚逻辑不能直接作为账号迁移回滚方案。
- 保留 `RELAY_REQUIRE_PROVISIONED_TOKENS=true`；不要删除 token 配置使系统退回可自行构造 `dev:<id>` 的开发身份模式。当前 token 文件加载器拒绝空映射，完全淘汰旧凭据时需要明确设计替代认证配置，不能直接用 `{}` 代替。

## 5. 容量、并发与小内存限制

下表按当前源码常量列出，没有对应的动态环境变量。MiB/KiB 均按二进制单位计算。

| 项目 | 当前限制与行为 |
| --- | --- |
| 账号 | 全库最多 1000 个；不是给每个账号预留磁盘空间。 |
| 主 SQLite 文件 | 512 MiB；按实际 page size 设置 `max_page_count`，4096 字节页时为 131072 页。超限新增失败，不自动删数据。 |
| WAL | 约 4 MiB 自动检查点阈值，`journal_size_limit` 为 4 MiB，`synchronous=FULL`。长期外部读事务可能推迟回收；不是 DB+WAL 总占用的硬配额。 |
| 历史 | 每个参与者最多 5000 条且最多 32 MiB；收发都计入自己的额度。存储层也对未注册旧收件人计量，但当前协议层拒绝新发混合消息。按加密序列化后的实际 payload 字节计量，含 Base64 和记录开销。 |
| 历史分页 | 仅参与者可取，按全局持久 seq 升序，每页最多 5 条。 |
| 表情库 | 每账号最多 20 个且总计最多 2 MiB，同样按加密 payload 计量，不是 2 MiB 原始图片。 |
| 媒体 | 每张 Base64 解码后的 PNG/JPEG 文件最多 256 KiB，真实宽高各最多 1024；实际解码核验格式和尺寸。解码器配置 32 MiB allocation limit，但并非进程内存硬保证。 |
| 文本 | 每条最多 2000 字符。 |
| 会话 | 每账号最多 8 个持久 token；同设备重新登录替换旧 token，超额会淘汰已有 token。在线连接也限制为每账号 8 个、全 relay 32 个。 |
| 网络缓冲 | 服务端入站消息/帧各最多 **1 MiB**；每连接出站队列最多 16 个事件，另有全局 **8 MiB 出站 payload 预算**，按事件 JSON 序列化大小加每事件 4096 字节开销预留。`QueuedEvent` 从入队到 writer 完成发送一直持有预算许可，发送失败或丢弃时释放；队满或预算不足时断连，由客户端补历史。服务端 1 MiB 接收限制不等于其出站表情列表上限。 |
| 云操作并发 | 全局 1 个云操作许可；Argon2 工作移至阻塞线程，在连接 Mutex 外执行，但其他云操作仍可能收到忙碌错误。 |
| 密码计算 | Argon2id，19 MiB / 2 次 / 1 lane；不是每次登录只占用 19 MiB 总进程内存。 |
| 云请求限流 | 注册/登录/恢复/Resume 合计每 IP 每 60 秒 30 次；其他云请求每账号每 60 秒 600 次。计数桶最多 4096 个、按窗口重置。并非全局 20 次/分钟，也不覆盖所有旧协议事件。 |

现有 unit 仍为 `MemoryLimit=128M`、`CPUQuota=25%`。出站同时受每连接事件数和全局 8 MiB payload 预算约束，预算覆盖已出队但尚未完成发送的事件；它不是进程 RSS 的硬上限，入站数据、入队前的事件副本、JSON/Base64 编解码、加密副本、数据库及网络缓冲仍会额外占用内存。**128 MiB 限制下的压力验收尚未完成**，不能据此保证最坏负载安全。上线前必须在相同资源限制下测试多设备图片同步、慢接收方、频繁恢复/登录与备份，观察峰值 RSS、cgroup OOM、忙碌率和 WAL/磁盘增长。不要未经验证就提高配额或承诺 1000 个账号的额定容量。

磁盘预算还须包含 WAL/SHM、配对 JSON、日志、备份与恢复临时副本。1000 × 32 MiB 不是可用容量承诺；可能先达到 512 MiB 主库上限。达到历史或表情上限后当前没有用户删除/清理 API，重试不会腾出空间，应作为运营容量事件处理。

认证限流使用连接来源 IP。当前代码仅在回环连接上接受 `X-ChickLink-Client-IP`；仓库专用 Caddy 配置会用真实连接地址**覆盖**这个头。若代理未同步此配置，多人可能共用回环 IP 的限额；不能允许客户端自行指定受信任来源头，也不应将 relay 回环端口对公网开放。本文件不代表该代理变更已在线部署。

## 6. 加密范围与上线前确认

云数据使用 `shared_protocol::SecureChannel` 的 AES-256-GCM 在服务端加密消息和表情 payload，并加密 key 校验标记。数据库不保存原始存储 key、明文密码、原始 session token 或恢复码；密码为 Argon2id 哈希，随机 token/恢复码以哈希保存。

**这不是端到端加密，也不是整个 SQLite 文件加密。** 服务端持有存储 key 和 RSA 私钥，处理请求时能看到内容；表结构、用户名、user ID、参与者关系、client ID、seq、设备标识、会话到期时间及哈希等元数据仍可从数据库读取。客户端本地记录、OS 缓存和备份也不因服务端 payload 加密自动获得保护。

未来生产启用前，需要独立确认：双方设备已升级、TLS 与公钥分发可信、测试身份迁移/撤销通过、独立备份和 key 恢复可用、128 MiB 负载验收完成、不会重新启用旧 token 的回滚路径已明确。当前没有注册邀请或管理员审核开关；公开启用后应按任何能连接的新版客户端均可尝试注册来评估账号配额消耗。

源码核对入口：

- `relay-server/src/cloud_service.rs`：启用条件、原 token 验证、限流、阻塞任务与云消息伙伴授权。
- `relay-server/src/main.rs`：RSA 格式、token 映射、身份声明检查、设备/连接/缓冲限制、监听及代理头处理。
- `relay-server/src/cloud.rs`：密码和恢复、key 校验、SQLite 事务、加密及容量常量。
- `shared-protocol/src/lib.rs`、`shared-protocol/src/cloud.rs`：AES-GCM 与账号协议；`pet-client/src/config.rs`：测试客户端环境变量；`pet-client/src/cloud_client.rs`：云设备身份和各平台数据目录。
- `deploy/standalone/chicklink-relay.service`、`deploy/standalone/Caddyfile`：仓库里的独立部署配置，不代表线上已应用。
