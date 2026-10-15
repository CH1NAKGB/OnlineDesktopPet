# 服务端快速部署

默认公网 TCP 18080，不占用宿主机 80、443、8080。容器内部仍使用 8080。
需要 x64 Linux、可用的 Docker Engine / Compose 和 OpenSSL。

解压服务端源码包，进入包含 Cargo.toml 的目录后运行：

```sh
sh deploy/relay.sh up
```

脚本检查 Docker、创建缺失的 `.env` 和 RSA-3072 密钥/随机令牌，构建并启动
容器，然后等待健康检查通过。首次构建需要下载 Rust 依赖，低配置机器可能
耗时较长。脚本不会自动安装 Docker 或修改系统防火墙。

## 低配置服务器：使用预编译镜像

预编译镜像必须与服务端源码包版本一致。v0.3.3 Release 已附带
`ChickLink-relay-linux-x64-v0.3.3.tar.gz`；使用它时须配套 v0.3.3 源码包。
v0.3.4 tag 的旧源码包仍把 Compose 镜像名写成 `0.3.3`，所以即使重新构建
v0.3.4 镜像，直接用旧包执行 `start` 也会找错标签。当前仓库已修复这两个脚本；
如仍使用旧 v0.3.4 源码包，请执行 `up` 从该包的源码构建，不要把新镜像改名成
`0.3.3` 来掩盖版本不匹配。

使用版本匹配且已修正部署脚本的源码包时，解压后进入目录运行：

```sh
docker load -i /镜像下载目录/ChickLink-relay-linux-x64-v对应版本.tar.gz
sh deploy/relay.sh start
```

`start` 自动初始化配置与凭据，但不进行源码编译；`up` 才会重新构建镜像。
升级预编译镜像时同样先导入新镜像，再使用新版源码包中的 `start`。
镜像不包含任何用户私钥和令牌，凭据在目标服务器上生成。

## 更换端口

```sh
sh deploy/relay.sh init
# 编辑 deploy/.env 中的 RELAY_PUBLIC_PORT，例如 18081
sh deploy/relay.sh up
```

在阿里云安全组和主机防火墙放行所选 TCP 端口。客户端地址为
`ws://服务器公网IP:18080/ws`。此配置强制 RSA/AES 应用层加密和预置令牌鉴权。

## 连接客户端

将 `deploy/secrets/relay-public.pem` 经可信渠道发给双方。
从 `deploy/secrets/client-tokens.env` 中分别取出各自令牌，只发给对应用户。
私钥和完整令牌表留在服务器上。不要将 secrets 或 .env 上传 GitHub。

macOS 终端启动已安装的客户端：

```sh
export CHICK_RELAY_URL='ws://服务器公网IP:18080/ws'
export CHICK_ACCESS_TOKEN='本人的令牌'
export CHICK_SERVER_RSA_PUBLIC_KEY_PATH='/公钥文件的绝对路径/relay-public.pem'
'/Applications/Chick Link.app/Contents/MacOS/Chick Link'
```

Windows PowerShell 在解压后的客户端目录运行：

```powershell
$env:CHICK_RELAY_URL = 'ws://服务器公网IP:18080/ws'
$env:CHICK_ACCESS_TOKEN = '本人的令牌'
$env:CHICK_SERVER_RSA_PUBLIC_KEY_PATH = 'C:\可信目录\relay-public.pem'
.\pet-client.exe
```

当前服务器配置通过进程环境传入；直接双击或系统开机启动不会自动继承
这些临时终端变量。关闭已运行的客户端后再用上述命令启动。

## 日常操作与升级

```sh
sh deploy/relay.sh status
sh deploy/relay.sh check
sh deploy/relay.sh logs
sh deploy/relay.sh stop
```

升级前备份 `deploy/.env` 与 `deploy/secrets/`，将新版源码解压到原目录，
再运行 `sh deploy/relay.sh up`。脚本保留现有凭据；发现不完整的密钥目录会停止。
不要删除密钥重新初始化，否则客户端固定公钥和登录令牌需要重新分发。

配对关系和离线通知当前保存在服务端内存，容器重建/重启后需要重新配对；
聊天历史保留在各客户端本地。健康检查只验证服务存活，公网访问还需真实客户端测试。

v0.3.3 镜像已在 GitHub Ubuntu Linux x64 runner 完成构建、启动和 HTTP 健康检查，
见 [构建记录](https://github.com/CH1NAKGB/OnlineDesktopPet/actions/runs/33964143902)。
实际阿里云/NAS 尚未部署，CentOS 7 的目标机兼容边界见 [centos7.md](centos7.md)。
项目整体接手信息见 [offread.md](../offread.md)；脚本 smoke test 的 fake Docker 不等同于此真实容器验证。
