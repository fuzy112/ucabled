# UI helper 进程契约（Contract v3）

> 状态：已实现。契约以 `src/bin/ucable-agent.rs`（发现与调用方）与
> `src/bin/ucable-agent-helper.rs`（自带参考实现）的当前行为为准。
> 本文只描述 agent 与 helper 之间的边界，不约束窗口内部的任何实现细节。
>
> 目标：把"QR 窗口"从具体 GUI 实现中解耦，使任何语言、任何 GUI
> 工具包都能替换 helper，而不必改动 agent。

## 1. 目的与插件模型

`ucable-agent` 是每用户会话里的 D-Bus agent，负责把守护进程的提示变成
用户可见的窗口。它自己不画 UI，而是为每次提示**新起一个 helper 子进程**，
通过标准输入输出完成一次会话。因此：

- helper 是一个**独立可执行文件**，不是动态库、不是插件 ABI；
- 可以用任意语言实现（本仓库自带的是 Rust/GTK4 版）；
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

找不到 helper 时，agent 不弹窗，只记一条 `tracing::warn!`：这次提示就这样
静默丢失，事务在守护进程/浏览器的时限到达后失败（blink 阶段守护进程回
`0x2C`，其余情况按各自的超时收场）。

NixOS 模块暴露 `services.ucabled.helper`，其值会作为 `UCABLED_HELPER` 注入每
用户会话 agent 的环境，从而在系统配置里指定替代 helper。

## 3. argv

```
<helper> [--rp <RP>] --timeout <SECS> [--request-type <mc|ga>]
```

- `--rp <RP>`：可选的**带值选项**，依赖方（relying party）域名字符串，
  **仅用于显示**（参考实现把它画在标题下方）。agent 只在其非空时传入。
  RP 来自 CTAP 载荷、内容不受信，因此必须以选项值的形式传递（参考实现
  也接受 `--rp=<RP>`），而不是位置参数——位置参数会让以 `-` 开头的值
  被误解析为标志位。
- `--timeout <SECS>`：十进制秒数，agent 原样透传守护进程给出的超时值。
- `--request-type <mc|ga>`：本次转发的 CTAP 命令，`mc` 对应
  `authenticatorMakeCredential`（在手机上保存通行密钥），`ga` 对应
  `authenticatorGetAssertion`（用通行密钥认证）。取值与 QR 内容里的
  request type（PXP §5.3.1 key 5）一致，agent 总是传入其中之一。
  helper 可据此说明扫码之后会发生什么；不认识这个参数（或只实现早期契约）
  的 helper 忽略即可，此时文案应退化为不涉及操作类型的说法。该值不是秘密，
  可以显示。
  在 Firefox 的 blink 选择阶段守护进程只能按探测本身填 `mc`，因此真实的
  getAssertion 也可能以 `mc` 到达；文案不应把 `mc` 当成承诺。

> **argv 整体仅用于显示。** argv 上的所有内容都是非秘密的展示数据
> （RP、超时、请求类型），任何本地用户都能通过 `/proc/<pid>/cmdline`
> 读到。caBLE URL **绝不**经 argv 传递（见 §4）：helper 必须忽略 argv
> 上任何形如 `FIDO:/…` 的参数，也不得提供任何从 argv 接收 URL 的
> 备用通道。

## 4. stdin

stdin 是管道。**第一行就是 caBLE URL**（`FIDO:/…`），它内含本事务的 16
字节 secret，因此**故意经管道传递**：它**绝不能**出现在 argv 或日志里。
第一行之后的行是 agent 发来的状态更新，目前只有 `found` 一条，含义是
"已收到手机发出的 BLE advertisement"；helper 应据此提示"已检测到手机"
（参考实现把 QR 模糊掉并改显 "Phone detected"）。

> 状态行以换行结尾，agent 在 `Prompt` 后立即写入 URL 行；`found` 可能在
> helper 启动前、窗口存续期间随时到达。

## 5. stdout / stderr

**不属于契约。** agent 不捕获也不解析 helper 的 stdout / stderr；写入
stdout 不会有任何效果，写入 stderr 的诊断信息会被忽略。helper 需要向
agent 传递的信息只有退出码（§6）。

## 6. 退出码

退出码是 helper 回传结果的唯一通道，且只有一种含义：进程退出即表示这次
UI 交互结束，agent 上报 `TransactionCancelled(tid)`，守护进程据此放弃该
事务（在 blink 选择阶段则回 `0x2C`，让实体密钥继续参与选择）。
具体取值不作区分。

> 被 agent 主动 `SIGKILL`（事务被取代或已关闭）的 helper，其退出不会被
> 上报（见 §8）。

## 7. 超时

`--timeout` 由 **helper 自己执行**，agent 不设看门狗、也不会在超时后杀
进程。参考实现把超时渲染为 "Code expired" 并等待用户关窗，并不自动退出。

> 新 helper 必须自己保证在时限内收场。若不自动退出，至少要在界面上明示
> 已过期，且不得让过期后的扫码继续被视为有效。

## 8. 生命周期

- 同一槽**一次只允许一个 helper**。
- 每次收到提示都**新起**一个 helper（无常驻复用）。
- 新提示到达、收到 `Close`、或依赖方要求关闭时，agent 对旧 helper 发
  `SIGKILL` 并**回收**它：进程由其窗口任务 `wait()` 收尸，被杀的 helper
  退出不会被上报。
- helper 正常退出后，仅当它仍是该槽的当前窗口时才会上报结果；被取代的
  退出会被抑制。

## 9. 版本

本文件描述 **Contract v3**。与 v2 的差异：

- 删除了 select 模式（`--select`、argv 上的选择窗口、退出码 0 = 用手机）。
  Firefox 的多设备选择现在由**同一个 QR 窗口**完成：扫码选中手机，触摸
  实体密钥则由浏览器取消 blink。`--select` 会被忽略。

与 v1 的差异（历史）：

- RP 由位置参数改为带值选项 `--rp <RP>`（v1 的 `[RP]` 会让以 `-` 开头
  的 RP 被当作标志位解析）；
- 删除了 v1 参考实现把 argv 上的 `FIDO:/…` 当作 URL 的历史容错：stdin
  现在是 URL 的唯一通道，helper 必须忽略 argv 上的 `FIDO:/` 参数。

> 建议后续在 argv 上增加 `--protocol-version <N>`（或等价的
> `--contract-version`），让 agent 在启动前声明契约版本，helper 可就地
> 拒绝不理解的版本而不是静默错行为。当前版本未实现该标志；在它落地前，
> 双方只能按 v3 假定兼容。

## 10. 实现一个新 helper 的清单

1. 接受 `<helper> [--rp <RP>] --timeout <SECS> [--request-type <mc|ga>]`；
   argv 仅用于显示，忽略一切不认识的参数（包括任何 `FIDO:/…`）。
2. 从 stdin 第一行读 URL；**不要**把 URL 写进 argv、日志、临时
   文件或任何持久位置，也不要接受 argv 上的 URL。
3. 继续消费 stdin 后续行，识别 `found` 并给出"检测到手机"的反馈。
4. 让用户能取消（关窗 / Esc / 按钮），并自行在 `--timeout` 后收场。
5. 假定同一槽内旧进程随时可能被 `SIGKILL`，不做跨进程的状态持久化。
6. 把 stdout / stderr 当作可丢弃的诊断，不依赖 agent 读取。

## 11. 社区/示例实现

- `contrib/ucable-agent-helper-foot`：极简 shell 版，用 foot 终端 +
  qrencode 显示 QR（ANSI 半方块），按键取消。依赖仅 foot、qrencode、
  bash；agent 的 stdin 经 0600 FIFO 转发进终端（foot 子进程拿的是 pty，
  读不到 agent 管道；FIFO 只过内核缓冲不落盘）。
