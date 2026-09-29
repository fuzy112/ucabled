# D-Bus 层异步化方案（dbus-tokio）

> 状态：方案待评审。动机是代码结构与健壮性，不是修复现存 bug——当前
> 同步 D-Bus 已被线程 + channel 完整隔离，功能上是正确的。

## 1. 现状

### 1.1 daemon（`ucabled`）

tokio 主循环之外，`src/agent.rs` 维持两块同步飞地：

| 线程 | 连接 | 职责 |
| --- | --- | --- |
| `ucable-agent-service` | 系统总线连接 #1 | crossroads 服务 `Manager1`；`RegisterAgent` 内嵌同步 polkit `CheckAuthorization`（每次注册再开一条短命连接 #3）；`NameOwnerChanged` 掉线监视 |
| `ucable-agent-dispatch` | 系统总线连接 #2 | 从 `mpsc<UiCommand>` 收命令，向 agent 发 `Prompt/Select/Found/Close`（15s 同步超时） |

与 tokio 侧的接口是三条 channel（`cancel_tx`、`select_tx`、`UiCommand`）
和跨三线程共享的 `Arc<Mutex<AgentSlot>>`。BlueZ 侧 bluer 内部已经跑
dbus-tokio（连接 #4），即同一进程里 libdbus 连接有四条、专用线程两个。

### 1.2 agent（`ucable-agent`）

完全同步：单连接 crossroads 服务 `Agent1`，主循环是
`conn.process(200ms)` + `try_recv` 排空上报队列 + `AtomicBool` 重注册
标志；每个窗口配一个 150ms sleep 轮询 `try_wait` 的 watcher 线程。

### 1.3 痛点

- 两套并发模型（tokio task / 阻塞线程 + channel + Mutex），状态分布在
  三处，顺序与生命周期要靠注释维持（"只有 service 线程能碰 conn"）。
- 轮询：agent 的 200ms `process` 循环、每窗口 150ms `try_wait` 线程。
- 超时语义硬编码在 libdbus 同步调用里（15s/5s/25s），无法与 tokio
  的 select/取消组合。
- 单连接多连接混用是历史遗留（同步 API 下一个连接一个线程），并非设计。

## 2. 目标 / 非目标

目标：

- daemon 的 D-Bus 服务、分发、掉线监视全部收进 tokio task，删除两个
  专用线程；agent 二进制整体 tokio 化，watcher 线程改为
  `tokio::process::Child::wait()`。
- D-Bus 接口（`Manager1`/`Agent1` 名字、方法签名、polkit 授权语义、
  唯一名校验规则）**逐字节不变**；对端无感知。
- 不新增依赖：dbus-tokio 0.7.6 已由 bluer 0.17.4 锁进依赖树。

非目标：

- 不换 zbus（进程内会有 libdbus + zbus 两套 D-Bus 栈，见 §6）。
- 不动 uhid / BLE / 隧道 / relay / session 逻辑；不动 eframe helper。
- 不改变并发语义：UI 命令仍串行分发，agent 仍单实例注册。

## 3. 方案（daemon 侧）

核心形态：**一条 `dbus::nonblock::SyncConnection` + 一个路由 task**。

```
new_system_sync() -> (IConnection 驱动, Arc<SyncConnection>, MessageStream)

路由 task（唯一拥有 crossroads 与 AgentSlot 的写路径）：
  for msg in stream {
      match msg {
          NameOwnerChanged(:1.x → "") => clear_if_matches(:1.x)
          RegisterAgent              => 见 §3.2
          其它 method_call           => crossroads.handle_message(msg, &conn)
      }
  }

UiClient 分发（原 dispatch 线程）：
  直接在 tokio 侧用 nonblock::Proxy.method_call(...).await
  + tokio::time::timeout(15s)，错误分类 agent_gone 逻辑照搬。
```

要点：

- dbus-tokio 0.7 的 `new_system_sync()` 给出 `SyncConnection`
  （实现 `dbus::channel::Sender`，可直接喂 crossroads）与消息流；
  具体 glue（spawn 驱动 `IConnection`、流的 filter 顺序）实现时核对，
  以编译为准。
- crossroads 是 `!Sync` 但只在路由 task 内使用，天然满足。
- `AgentSlot` 仍是 `Arc<Mutex<_>>`（`UiClient.available()` 是同步查询）；
  规则：**锁不得跨过 `.await`**——只在 await 前克隆出 `Agent` 即放锁，
  与今天 dispatch 线程的用法相同。
- `NameOwnerChanged` 不再用 `add_match` 回调，改为在路由 task 里对信号
  分流（先 `add_match_no_cb` 注册匹配规则）。
- 连接数：daemon 的 D-Bus 连接从 4 条（service/dispatch/短命 polkit/bluer）
  降为 2 条（本方案 1 条 + bluer 1 条）。

### 3.1 出站调用的取消语义

今天 `Prompt` 的 15s 是 libdbus 同步超时，期间 dispatch 线程被占。
异步化后 `tokio::time::timeout` 包裹 future；**放弃等待不等于取消
D-Bus 调用**（libdbus 会丢弃迟到的应答），语义与今天一致，无需额外处理。

### 3.2 `RegisterAgent` 的 polkit 检查

crossroads 0.5 的方法是同步闭包，无法原生 async。两个选项：

- **选定：保留同步 `authorize()`，内嵌在闭包里**（照今天做法，短命
  阻塞连接）。注册是稀有事件，polkitd 本地应答在毫秒级；25s 只是超时
  上限。代价是路由 task 在注册瞬间短暂停摆——可接受，且与今天
  service 线程被占完全等价。
- 备选：在路由 task 里把 `RegisterAgent` 从 crossroads 摘出来手写，
  spawn 独立 task 做异步 polkit 调用再手动回包。更干净但丢掉
  crossroads 的参数解析，复杂度不值。

### 3.3 `UiClient` 与 channel

`start()` 不再 spawn 线程，改为在已有 runtime 上 spawn task，因此签名
从自由函数变为需要 `&tokio::runtime::Handle`（或在 `daemon_loop` 内直接
构造）。`cancel_tx`/`select_tx` 通道不变；`UiCommand` 的 mpsc 可保留
（`available()` 等同步查询路径不变），也可顺手换成直接持
`SyncConnection` 句柄——以最小 diff 为先，保留 mpsc。

## 4. 方案（agent 侧）

- main 改 `#[tokio::main]`；单条 `SyncConnection`，路由 task 喂
  crossroads（同 §3 形态）。
- 上报（`TransactionCancelled`/`SelectionResult`）与注册
  （`RegisterAgent`）改为同连接 nonblock 调用；唯一名身份由同连接
  天然保证（今天的"必须用服务连接上报"约束自动满足）。
- 重注册：`NameOwnerChanged(name=org.ucabled, new_owner≠"")` 进路由
  task 置标志，主循环 `tokio::time::interval(2s)` 重试——删除
  `AtomicBool` + 200ms 轮询。
- helper 子进程：`tokio::process::Command`，`Child::wait()` 直接
  await 退出，删除两个 150ms watcher 线程；写 stdin 与 kill 语义不变
  （`Close` 时 `kill().await`）。
- 无图形会话直接退出、被拒（`NotAuthorized`）退出等行为不变。

## 5. 验证

- `cargo test`：现有 39+2 单测中 D-Bus 相关的是 `AgentSlot` 状态机与
  `agent_gone` 分类，均不触连接层，预期零改动通过；为"锁不跨 await"
  与路由分流补 1–2 个单测。
- 真机/真会话验收（必须，覆盖单测到不了的 D-Bus 行为）：
  1. 正常 QR 注册 + 登录（Firefox）；
  2. 关窗取消路径（helper 退出 → `TransactionCancelled` → 0x2d）；
  3. 实体密钥共存的选择窗口（`Select` → `SelectionResult` 两路）；
  4. agent 掉线（`kill` agent，事务按取消失败处理）与 daemon 重启后
     agent 自动重注册；
  5. `--no-ui` 回落不受影响。

## 6. 否决方案

- **zbus**：异步体验最好，但 bluer 锁死 libdbus，引入 zbus = 进程内
  两套 D-Bus 实现（两套连接、两套类型系统，polkit 结构体要用 zvariant
  重声明）。当初"不引 zbus"的理由仍成立。
- **保持现状**：功能正确；本重构的价值是删掉 §1.3 的并发模型分裂与
  轮询。若评审认为收益不足，本方案可整体搁置，无沉没成本。

## 7. 提交拆分

1. daemon：路由 task 取代 service 线程（含 §3.2 的同步 authorize 保留）。
2. daemon：`UiClient` 出站调用异步化，删除 dispatch 线程。
3. agent：整体 tokio 化（§4），删除 watcher 线程与轮询。
4. 文档：`docs/system-service.md` §3/§7 的运行模型描述、README 架构
   图注释同步；本文件标记"已实现"。
