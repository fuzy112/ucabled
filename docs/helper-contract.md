# UI helper 进程契约（Contract v1）

> 状态：已实现。契约以 `src/bin/ucable-agent.rs`（发现与调用方）与
> `src/bin/ucable-agent-helper.rs`（自带参考实现）的当前行为为准。
> 本文只描述 agent 与 helper 之间的边界，不约束窗口内部的任何实现细节。
>
> 目标：把"QR / 设备选择窗口"从 egui 实现中解耦，使任何语言、任何 GUI
> 工具包都能替换 helper，而不必改动 agent。

## 1. 目的与插件模型

`ucable-agent` 是每用户会话里的 D-Bus agent，负责把守护进程的提示变成
用户可见的窗口。它自己不画 UI，而是为每次提示**新起一个 helper 子进程**，
通过标准输入输出完成一次会话。因此：

- helper 是一个**独立可执行文件**，不是动态库、不是插件 ABI；
- 可以用任意语言实现（本仓库自带的是 Rust/egui 版）；
- 一次提示对应一个进程，进程退出即代表这次 UI 交互结束；
- agent 只依赖 argv、stdin、退出码三项，其余（窗口框架、主题、渲染）
  完全自由。

这层边界存在的意义，是让 UI 后端可替换——终端 QR、原生窗口、甚至测试
替身都能挂在同一契约上。

## 2. 发现（Discovery）

agent 按以下顺序解析 helper 可执行文件：

| 顺序 | 来源 | 行为 |
| --- | --- | --- |
| 1 | 环境变量 `UCABLED_HELPER` 为**绝对路径** | 路径存在则使用；不存在则 `tracing::warn!` 并继续第 3 步 |
| 2 | 环境变量 `UCABLED_HELPER` 为**裸名字**（非绝对路径） | 原样作为程序名返回，交由 `Command` 按 `PATH` 解析 |
| 3 | 默认 | agent 二进制同目录下的 `ucable-agent-helper`；不存在则视为无 helper |

> `UCABLED_HELPER` 为空串时告警并按默认路径处理。
>
> 裸名字意味着 helper 必须位于会话 agent 的 `PATH` 中；绝对路径则是
> 自包含部署（Nix、Flatpak、容器）的推荐形态。

找不到 helper 时，agent 不弹窗：QR 模式直接放弃该事务，select 模式按
"放弃、保留实体密钥"作答，两者各记一条 `tracing::warn!`。

## 3. 模式与 argv

helper 有两种模式，由是否存在 `--select` 区分。`--timeout <SECS>` 两种
模式都会传入。

### 3.1 QR 模式

```
<helper> [RP] --timeout <SECS>
```

- `RP`：可选的位置参数，依赖方（relying party）域名字符串，**仅用于显示**
  （参考实现把它画在标题下方）。agent 只在其非空时传入。
- `--timeout <SECS>`：十进制秒数，agent 原样透传守护进程给出的超时值。

### 3.2 Select 模式

```
<helper> --select --timeout <SECS>
```

- 用于"检测到另一把安全密钥，是否改用手机"的选择窗口。
- 没有 RP 位置参数。

> 参考实现的解析器还把 argv 中以 `FIDO:/` 开头的参数当作 QR URL。这只是
> 历史容错：按契约 URL 必须走 stdin（见 §4），agent 从不在 argv 里传 URL。

## 4. stdin

stdin 的行为按模式不同：

- **QR 模式**：stdin 是管道。**第一行就是 caBLE URL**（`FIDO:/…`），它
  内含本事务的 16 字节 secret，因此**故意经管道传递**：它**绝不能**出现在
  argv 或日志里。第一行之后的行是 agent 发来的状态更新，目前只有 `found`
  一条，含义是"已收到手机发出的 BLE advertisement"；helper 应据此提示
  "已检测到手机"（参考实现把 QR 模糊掉并改显 "Phone detected"）。
- **Select 模式**：stdin 接 `/dev/null`，helper 不应读取。

> 状态行以换行结尾，agent 在 `Prompt` 后立即写入 URL 行；`found` 可能在
> helper 启动前、窗口存续期间随时到达。

## 5. stdout / stderr

**不属于契约。** agent 不捕获也不解析 helper 的 stdout / stderr；写入
stdout 不会有任何效果，写入 stderr 的诊断信息会被忽略。helper 需要向
agent 传递的信息只有退出码（§6）。

## 6. 退出码

退出码是 helper 回传结果的唯一通道。

| 退出码 | QR 模式 | Select 模式 |
| --- | --- | --- |
| `0` | 肯定：该窗口是当前存活的窗口，交互正常结束；agent 随即取消该事务 | 用户选择"用手机" |
| 非 `0` | 取消 / 关窗 / 超时 / 被信号杀死 | 放弃，保留实体密钥（保留其它密钥） |

> 当前 agent 在 QR 模式**不区分**退出码：只要 helper 退出，就上报
> `TransactionCancelled(tid)`（参考实现的 QR 路径也只会退出 0）。非 0 的
> 语义主要是为 select 模式定义、并为将来可能的区分预留。
>
> 被 agent 主动 `SIGKILL`（事务被取代或已关闭）的 helper，其退出不会被
> 上报（见 §8）。

## 7. 超时

`--timeout` 由 **helper 自己执行**，agent 不设看门狗、也不会在超时后杀进程。

- Select 模式：到点后立即以非 0（参考实现为 `EXIT_DECLINE = 1`）退出。
- QR 模式：参考实现把超时渲染为 "Code expired" 并等待用户关窗，并不自动
  退出；此时退出码仍为 0。

> 新 helper 必须自己保证在时限内退出。QR 模式若不自动退出，至少要在界面上
> 明示已过期，且不得让过期后的扫码继续被视为有效。

## 8. 生命周期

- QR 与 select 各有一个窗口槽，同一槽**一次只允许一个 helper**。
- 每次收到提示都**新起**一个 helper（无常驻复用）。
- 新提示到达、收到 `Close`、或依赖方要求关闭时，agent 对旧 helper 发
  `SIGKILL` 并**回收**它：进程由其窗口任务 `wait()` 收尸，被杀的 helper
  退出不会被上报。
- helper 正常退出后，仅当它仍是该槽的当前窗口时才会上报结果；被取代的
  退出会被抑制。

## 9. 版本

本文件描述 **Contract v1**。

> 建议后续在 argv 上增加 `--protocol-version <N>`（或等价的
> `--contract-version`），让 agent 在启动前声明契约版本，helper 可就地
> 拒绝不理解的版本而不是静默错行为。当前版本未实现该标志；在它落地前，
> 双方只能按 v1 假定兼容。

## 10. 实现一个新 helper 的清单

1. 接受 `<helper> [RP] --timeout <SECS>`（QR）与
   `<helper> --select --timeout <SECS>`（选择）两种调用。
2. QR 模式下从 stdin 第一行读 URL；**不要**把 URL 写进 argv、日志、临时
   文件或任何持久位置。
3. QR 模式继续消费 stdin 后续行，识别 `found` 并给出"检测到手机"的反馈。
4. QR 模式让用户能取消（关窗 / Esc / 按钮），并自行在 `--timeout` 后收场。
5. Select 模式不读 stdin，提供"用手机"与"保留安全密钥"两条路径。
6. 用退出码回传结果：select 模式 0 = 用手机，非 0 = 放弃；QR 模式正常
   结束用 0。
7. 假定同一槽内旧进程随时可能被 `SIGKILL`，不做跨进程的状态持久化。
8. 把 stdout / stderr 当作可丢弃的诊断，不依赖 agent 读取。
