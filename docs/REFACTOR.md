# WFTPD v3 重构设计

本文档记录 v3.0.0 重构的两项架构决策评估（前后端通信方案、FTP/SFTP 进程模型），
以及落地后的工作空间结构与运行模型。

---

## 一、前后端通信方案评估（gRPC over UDS vs 纯文件）

### 背景

前后端均以当前桌面登录用户运行，理论上"前端直接改后端配置文件 + 监听日志文件变化"
在权限上完全可行。评估结论是：**保留 gRPC(tonic) over UDS 作为控制面，
文件只作为持久化载体**。

### 两种方案对比

| 维度 | gRPC(tonic) + UDS | 纯文件（改配置文件 + tail 日志文件） |
|------|-------------------|--------------------------------------|
| 配置保存反馈 | 后端解析→校验→落盘→应用，错误信息即时回传前端 | 前端写入后"祈祷"后端能解析；无校验回路 |
| 运行时状态 | 服务是否运行、版本、活动连接等实时可查 | 只能靠 PID 文件/端口探测间接推断，不可靠 |
| 实时日志 | WatchLogs 服务端流式推送，无轮询 | 需要 inotify/轮询；日志按天滚动时文件名变化、轮转瞬间可能丢事件，边界情况多 |
| 配置热更新 | SaveConfig 后后端原子替换内存配置并应用 enabled 标志 | 需要约定"写临时文件+rename"原子写协议和重载信号（SIGHUP），本质是重新发明 IPC |
| 类型安全 | proto 生成类型 + 共享 `wftpd-common` crate，前后端永不漂移 | 前端必须复制一份 config/users 解析代码（重构前 wftpg 已经与 wftpd 漂移：`enable_gui_logging` 字段两边不一致） |
| 访问控制 | UDS 套接字 0600，同机其他用户无法连接 | 需要仔细管理每个文件的权限，多用户桌面上日志/配置可能被其他用户读取 |
| 写入单一性 | 所有写操作经由后端，天然避免并发写坏文件 | 前后端都可能写 users.json/config.toml，必须自建文件锁 |
| 代价 | 引入 tonic/prost 依赖；proto 代码生成（已用 protox 规避 protoc 系统依赖） | 无 IPC 代码，但把复杂度转移到文件协议与轮询逻辑上 |

### 结论

采用**混合模型**：

- **控制面走 gRPC**：配置/用户 CRUD（带校验回执）、服务启停/重启、状态查询、
  内存日志读取与流式订阅、审计日志写入、目录辅助操作。后端是唯一的文件写入者。
- **数据面落文件**：`config.toml` / `users.json` 仍是持久化的唯一事实来源（后端独占写入）；
  日志照常落盘（tracing-appender 按天滚动），便于脱离 GUI 用任意工具排查。
- **契约即代码**：`proto/proto/wftpd.proto` 是前后端唯一契约，`wftpd-proto` crate
  同时提供生成类型与领域类型转换；前端不再拥有任何配置解析副本。

### 为什么 tonic + UDS 而不是 JSON 行协议（旧方案）

旧 IPC（JSON lines over UDS）的问题：协议结构（IpcCommand/IpcResult 枚举）在前后端
各维护一份、手写序列化、无流式能力、后端侧实现实际缺失（重构前 wftpd 根本没有 IPC
服务端，GUI 的日志/配置接口全部不可用）。tonic 带来：强类型双向契约、原生服务端流
（WatchLogs）、超时/取消语义、生态工具（grpcurl 等可直接调试）。

---

## 二、FTP/SFTP 进程模型评估

### 三个候选方案

**A. 单进程双任务（当前采用）**：`wftpd` 一个进程内 FTP/SFTP 各为一个 tokio 任务。

- 优点：一个 systemd 用户服务单元、一个控制套接字、一份内存状态（config/users/quota）；
  FTP/SFTP 已支持运行期独立启停（StartService/StopService per protocol），重启其一
  不影响另一协议的活跃连接；故障恢复由 systemd `Restart=on-failure` 兜底。
- 缺点：panic=abort 下任一协议的 panic 会拖垮整个进程（另一协议的连接同时断开，
  systemd 随后拉起）；两个协议共享同一进程内存，安全隔离弱。

**B. 监督进程 + 两个子进程**：一个服务管理两个 worker 进程。

- 优点：故障隔离；单协议可独立重启。
- 缺点：需要自研进程监督（spawn、僵尸回收、退避重启、崩溃计数）——这些 systemd
  已经做了；控制面 gRPC 需要挂在监督进程，再向 worker 转发或各自暴露，复杂度显著
  上升；两个 worker 共享 users.json 还要额外做文件锁协调。

**C. 两个独立 systemd 用户服务**（wftpd-ftp.service + wftpd-sftp.service，同一二进制
不同子命令）。

- 优点：故障域与代码边界完全对齐（ftp/sftp 本来就是独立 crate）；systemd 原生独立
  启停/重启；每服务可设独立资源限制。
- 缺点：两个控制套接字，前端需要分别连接聚合状态；配置保存后要通知两个服务重载；
  users.json 双写需要文件锁（fs2 已具备但需要正确使用）。

### 结论

**采用方案 A**，理由：

1. 桌面场景下部署单元最简（一条 `systemctl --user enable --now wftpd` 即可用），
   与"前后端都以桌面用户运行"的整体模型一致；
2. FTP/SFTP 的独立启停需求已由进程内任务级启停满足（本次重构顺带修复了重启时
   监听套接字未释放导致 Address already in use 的问题）；
3. panic 隔离的收益在该规模下有限，而 systemd 兜底重启已覆盖主要故障场景。

**保留升级路径**：ftp/sftp 是完全独立的库 crate，不依赖 wftpd 的进程结构。若未来
需要强故障隔离，切换到方案 C 只是打包层工作（两个 unit + `wftpd --ftp`/`wftpd --sftp`
子命令 + 前端聚合两个套接字），协议与库代码无需改动。

---

## 二点五、协议实现切换为成熟库（libunftp / russh-sftp）

v3.0.0 首轮重构后，FTP 与 SFTP 协议仍是手写实现（约 4000 行协议代码）。第二轮重构
将协议层替换为成熟库：

| 协议 | 之前 | 现在 |
|------|------|------|
| FTP/FTPS | 手写命令解析（`commands/` + `handler` + `data_connection` + `tls`） | **libunftp 0.23**（FTPS 内置，`ring` crypto provider） |
| SFTP | russh 传输层 + 手写包编解码（`packet.rs` + `state.rs` 1793 行 + `extensions.rs`） | **russh 0.59**（传输层不变）+ **russh-sftp 2.4**（SFTP 协议层） |

### 替换方式

FTP（`ftp/` crate，协议代码从 ~2200 行缩减到 ~600 行）：

- `auth.rs`：`unftp_core::auth::Authenticator` 桥——argon2 密码校验（认证前重载
  users.json）、匿名访问、IP 白/黑名单（libunftp 的 `Credentials` 自带 `source_ip`）；
  `UserDetailProvider` 把认证主体映射为携带主目录/权限/配额的 `WftpdUser`
- `storage.rs`：`StorageBackend` 实现——`enter()` 在登录时把会话根切到用户主目录
  （等价 chroot，路径词法规范化并拒绝 `..` 越界），操作级权限检查、配额、审计日志
- `server.rs`：libunftp `ServerBuilder` 组装——passive ports/host、greeting、
  idle timeout、FTPS（`ftps()` + `ftps_required()`）、防爆破
  （`FailedLoginsPolicy`，UserAndIP 维度）、`shutdown_indicator` 优雅关停

SFTP（`sftp/` crate，协议代码从 ~2600 行缩减到 ~700 行）：

- `handler.rs`：russh `server::Handler`——密码/公钥认证（保留原逻辑与审计），
  `subsystem_request("sftp")` 时把通道 `into_stream()` 交给 `russh_sftp::server::run`
- `ops.rs`：`russh_sftp::server::Handler` 实现——路径词法解析（chroot、不跟随
  符号链接）、权限/配额/限速/审计，`md5sum` / `sha256sum` / `space-available`
  openssh 扩展，`realpath` 返回 chroot 内虚拟路径

### 功能与行为变化

**增强**：SFTP 的 SYMLINK / READLINK / RENAME 不再是"不支持"（E2E 从 11/15 → 15/15）；
FTPS 支持（显式 AUTH TLS，证书可配）不再依赖手写 TLS 状态机。

**有意的行为对齐**（与旧实现对齐而非"纠正"）：
- RMD/RMDIR 递归删除（`remove_dir_all`），与旧实现一致
- SFTP symlink 参数按实际生态（paramiko/OpenSSH）顺序处理（第一参数=目标，
  第二参数=链接位置），与 SFTP 规范相反
- 符号链接目标按 POSIX 语义保存原样字符串，但创建时校验解析后不越出主目录

**降级**（libunftp 当前未提供对应钩子）：
- `security.max_connections` 不再强制（libunftp listen 自管 accept，无连接数上限钩子）
- `ftp.max_speed_kbps` 不再对 FTP 生效（SFTP 的用户级 `speed_limit_kbps` 仍生效）

### 验证结果

- E2E 协议回归：**FTP 17/17 + SFTP 15/15 = 32/32（100%）**，超越旧手写实现基线
  （旧 SFTP 11/15，SYMLINK/READLINK/RENAME/CHMOD 均失败）
- gRPC 全接口探测（`grpc_probe`）通过；`cargo test` 全绿；clippy/fmt 干净

---

## 三、工作空间结构（落地结果）

```
wftpd-linux/
├── Cargo.toml          # workspace 根：成员、共享依赖版本、release profile
├── common/             # wftpd-common  —— 配置/用户/日志/XDG 路径/服务端公共工具
├── proto/              # wftpd-proto   —— gRPC 契约（protox 编译，无需 protoc）
├── ftp/                # wftpd-ftp     —— FTP 服务器（仅依赖 common）
├── sftp/               # wftpd-sftp    —— SFTP 服务器（仅依赖 common）
├── wftpd/              # bin wftpd     —— 后端：组装 FTP+SFTP + gRPC 控制服务(UDS)
└── wftpg/              # bin wftp-gui  —— 前端：GTK3 配置管理/服务控制/日志查看
```

依赖方向：`wftpd → {ftp, sftp, proto, common}`，`wftpg → {proto, common}`，
`ftp|sftp → common`，`proto → common`。前后端不再有任何复制的业务代码。

## 四、运行模型与路径（用户态）

| 用途 | 路径 |
|------|------|
| 配置 | `$XDG_CONFIG_HOME/wftpd/config.toml`（默认 `~/.config/wftpd/`） |
| 用户库 | `$XDG_CONFIG_HOME/wftpd/users.json` |
| 公钥目录 | `$XDG_CONFIG_HOME/wftpd/keys/<user>/authorized_keys` |
| 日志/密钥 | `$XDG_STATE_HOME/wftpd/`（默认 `~/.local/state/wftpd/`） |
| gRPC 套接字 | `$XDG_RUNTIME_DIR/wftpd/wftpd.sock`（0600） |

- 环境变量 `WFTPD_CONFIG_DIR` / `WFTPD_STATE_DIR` 可覆盖配置与状态目录（测试/多实例）。
- 服务单元安装至 `/usr/lib/systemd/user/wftpd.service`，用户执行
  `systemctl --user enable --now wftpd` 启用；GUI 的"系统服务"页提供按钮化的等价操作。
- 旧的 `/etc/wftpg`、`/var/lib/wftpg`、`/var/log/wftpg`、专用 `wftpg` 用户、
  pkexec 提权策略全部移除。

## 五、gRPC 接口一览（wftpd.v1.Control）

- 生命周期：`GetStatus` / `StartService` / `StopService` / `RestartService`
  （Start/Stop 会同步持久化 enabled 标志，保证重启后意图一致）
- 配置与用户：`GetConfig` / `SaveConfig`（先校验后落盘，失败返回原因）
  / `GetUsers` / `SaveUsers`（临时文件校验后原子替换）/ `GetInitialState`
- 目录辅助：`EnsureUserDirectories` / `CreateUserDirectory` / `SetupDirectoryPermissions`
- 日志：`GetRecentLogs`（内存环形缓冲）/ `WatchLogs`（服务端流：先推历史再推实时）
  / `GetLogFiles` / `GetLogFileContent`（程序日志）/ `GetFileOpLogFiles`
  / `GetFileOpLogContent`（文件操作审计，`path="current"` 读内存缓冲）
  / `SaveLogConfig` / `WriteAuditLog`

诊断工具：`cargo run -p wftpg --example grpc_probe` 可对运行中的后端做全接口探测。

## 六、重构期间顺带修复的问题

1. **SFTP stop 不生效**：旧实现的 shutdown 信号从未真正中断 russh 的 accept 循环，
   导致服务"停止"后端口仍被占用。现改为 select 中断并显式 drop。
2. **重启端口冲突**：FTP/SFTP 重启时旧监听套接字尚未释放就重新 bind，
   报 `Address already in use`。现 stop() 会等待"监听已释放"信号后再返回。
3. **配置类型漂移**：GUI 侧 Config 副本多出 `enable_gui_logging` 字段而后端缺失
   （serde 宽松解析掩盖了不一致）。统一到 common 后消除。
4. **主机密钥命名不一致**：默认路径名为 `ssh_host_rsa_key` 但实际生成 Ed25519 密钥。
   现统一为 `ssh/ssh_host_ed25519_key`。
5. **`static mut` 违规**：tracing reload 句柄使用 `static mut`（edition 2024 禁止），
   改为 `OnceLock`。
6. **前端 IPC 实为空转**：旧客户端连接的 UDS 服务端在后端根本不存在，
   GUI 的配置保存/日志查看全部失效；gRPC 控制面补齐了这条链路。

## 七、遗留事项

- SFTP 的 CHMOD/RENAME/SYMLINK/READLINK 四项与重构前基线一致地失败
  （见 `test_result.json`），属于既有协议实现问题，与本次重构无关，待后续单独修复。
- deb 打包已适配用户服务模型；`lintian` 提示项可在发布前统一处理。

---

## 二点五、协议实现切换为成熟库（libunftp / russh-sftp）

v3.0.0 首轮重构后，FTP 与 SFTP 协议仍是手写实现（约 4000 行协议代码）。第二轮重构
将协议层替换为成熟库：

| 协议 | 之前 | 现在 |
|------|------|------|
| FTP/FTPS | 手写命令解析（`commands/` + `handler` + `data_connection` + `tls`） | **libunftp 0.23**（FTPS 内置，`ring` crypto provider） |
| SFTP | russh 传输层 + 手写包编解码（`packet.rs` + `state.rs` 1793 行 + `extensions.rs`） | **russh 0.59**（传输层不变）+ **russh-sftp 2.4**（SFTP 协议层） |

### 替换方式

FTP（`ftp/` crate，协议代码从约 2200 行缩减到约 600 行）：

- `auth.rs`：`unftp_core::auth::Authenticator` 桥——argon2 密码校验（认证前重载
  users.json）、匿名访问、IP 白/黑名单（libunftp 的 `Credentials` 自带 `source_ip`）；
  `UserDetailProvider` 把认证主体映射为携带主目录/权限/配额的 `WftpdUser`
- `storage.rs`：`StorageBackend` 实现——`enter()` 在登录时把会话根切到用户主目录
  （等价 chroot，路径词法规范化并拒绝 `..` 越界），操作级权限检查、配额、审计日志
- `server.rs`：libunftp `ServerBuilder` 组装——passive ports/host、greeting、
  idle timeout、FTPS（`ftps()` + `ftps_required()`）、防爆破
  （`FailedLoginsPolicy`，UserAndIP 维度）、`shutdown_indicator` 优雅关停

SFTP（`sftp/` crate，协议代码从约 2600 行缩减到约 700 行）：

- `handler.rs`：russh `server::Handler`——密码/公钥认证（保留原逻辑与审计），
  `subsystem_request("sftp")` 时把通道 `into_stream()` 交给 `russh_sftp::server::run`
- `ops.rs`：`russh_sftp::server::Handler` 实现——路径词法解析（chroot、不跟随
  符号链接）、权限/配额/限速/审计，`md5sum` / `sha256sum` / `space-available`
  openssh 扩展，`realpath` 返回 chroot 内虚拟路径

### 功能与行为变化

增强：

- SFTP 的 SYMLINK / READLINK / RENAME 不再"不支持"，且 E2E 从 11/15 升至 15/15
- FTPS 支持（显式 AUTH TLS，证书可配、可强制）

有意的行为对齐（与旧实现对齐而非"纠正"）：

- RMD/RMDIR 递归删除（`remove_dir_all`），与旧实现一致
- SFTP symlink 参数按实际生态（paramiko/OpenSSH）顺序处理：第一参数=目标，
  第二参数=链接位置（与 SFTP 规范相反，属 paramiko 的已知兼容性怪癖）
- 符号链接目标按 POSIX 语义保存原样字符串，但创建时校验解析后不越出主目录

降级（libunftp 当前未提供对应钩子）：

- `security.max_connections` 不再强制（libunftp listen 自管 accept，无连接数上限钩子）
- `ftp.max_speed_kbps` 不再对 FTP 生效（SFTP 的用户级 `speed_limit_kbps` 仍生效）

### 验证结果

- E2E 协议回归：FTP 17/17 + SFTP 15/15 = 32/32（100%），超越旧手写实现基线
  （旧 SFTP 11/15：SYMLINK/READLINK/RENAME/CHMOD 均失败）
- gRPC 全接口探测（`grpc_probe`）通过；`cargo test` 全绿；clippy/fmt 干净
