# 系统服务与用户 UI Agent 设计（M2/M3 修订）

> 状态：已实现。代码见 `src/agent.rs`（D-Bus 层与 polkit 授权）、
> `src/bin/ucable-agent.rs`（session agent）、`src/bin/ucabled.rs`（daemon
> 接线）与 `nix/module.nix` / `dist/`（打包）。
> 本文取代 `docs/plan.md` 中"systemd user service"的运行模型，以及 M2/M3
> 两次临时缓解（udev `uaccess` + user unit sandbox）。
>
> 目标：守护进程以**独立系统用户**运行并独占 `/dev/uhid`；UI 由每个用户
> 会话里的 agent 通过 **system D-Bus** 提供；用 **polkit** 保证只有**当前
> 活动本地会话的用户**能注册 agent。

## 1. 背景与动机

当前实现里 `ucabled` 是一个 **systemd user service**，以登录用户身份运行，
通过 `/dev/uhid` 的 `TAG+="uaccess"` 打开设备。由此：

- **M2**：`/dev/uhid` 是通用内核接口，`uaccess` 意味着该用户的**任意进程**
  都能创建虚拟 HID 设备（包括虚拟键盘，即输入注入），并非仅限本项目的
  FIDO 设备。
- **M3**：sandbox 只能加在 user unit 上，且 QR 子进程与守护进程共享同一
  安全上下文。

关键观察：Firefox 打开的是内核生成的 **hidraw** 节点（由 systemd 的
`60-fido-id.rules` + `70-uaccess.rules` 给 seat 用户 uaccess），**不需要**
访问 `/dev/uhid`。因此把守护进程搬出用户会话，就能在不影响 Firefox 的前
提下收回"创建虚拟 HID 设备"的能力。

唯一阻碍是 **QR 窗口必须显示在用户的图形会话里**——系统服务没有
Wayland/X11 会话。解决办法是拆出一个每用户的 UI agent。

## 2. 目标架构

```
数据通路
 ┌─────────┐  hidraw ┌─────────────────┐  caBLE v2 ┌──────────────────┐
 │ Firefox │────────>│ ucabled.service │──────────>│ iPhone / Android │
 └─────────┘         └─────────────────┘           └──────────────────┘
（hidraw = CTAP-HID / /dev/uhid；caBLE v2 = BLE advert + WSS 隧道）

控制通路（system D-Bus，org.ucabled；ucabled 实现 Manager1，对象 /org/ucabled/Manager）
 ┌───────────────┐             ┌─────────────────┐
 │ ucable-agent  │────────────>│ ucabled.service │ RegisterAgent / TransactionCancelled
 │ （user unit） │<────────────│ （system unit） │ Prompt / Found / Close
 └───────┬───────┘             └─────────────────┘
         │  spawn（QR URL 经 stdin）
         V
 ┌─────────────────────┐
 │ ucable-agent-helper │  QR 窗口
 └─────────────────────┘
```

`ucable-agent-helper`、`DesktopFlow`、Noise/隧道、BLE 扫描等既有实现不变；
`run_qr_transaction` 的 closure 接口保持，只是 `on_qr`/`on_advert`/`hide`
从"进程内 spawn GUI"变成"经 D-Bus 通知 agent"。

## 3. 组件

### 3.1 系统守护进程 `ucabled.service`

- system unit，`User=ucabled`、`Group=ucabled`（专用系统账号）。
- 依赖：`/dev/uhid`（udev ACL `setfacl -m u:ucabled:rw`，不改节点组）、BlueZ（system
  bus）、网络（WSS 隧道）。
- 拥有 system bus 名 `org.ucabled`（暂定），对象 `/org/ucabled/Manager`，实现
  接口 `org.ucabled.Manager1`。
- 仍然负责 HID 收发、U2FHID 分片、CTAP 分发、caBLE 握手与中继；**不再
  spawn 任何 GUI 子进程**。
- 无可用 agent 时，对 `MakeCredential`/`GetAssertion` 直接回 CTAP 错误
  并记日志，不启动一个无人可见的 QR 事务。

### 3.2 用户 agent `ucable-agent.service`

- user unit，对所有用户启用；谁真正能弹窗完全由 polkit 决定，不按用户配置。
- 连接到 **system bus**，导出一个对象实现 `org.ucabled.Agent1`，调用
  `org.ucabled.Manager1.RegisterAgent(path)` 注册自己。
- 收到 `Prompt(tid, url, rp, timeout)` 时，按既有窗口逻辑 spawn
  `ucable-agent-helper`，把 URL 写进它的 stdin；把 helper 的退出转成
  `TransactionCancelled(tid)`；收到 `Found`/`Close` 时更新/关闭窗口。
- 收到 `Select(tid)` 时 spawn `ucable-agent-helper --select`，按退出码
  （0=用手机，其余=放弃）回 `SelectionResult(tid, use_phone)`。
- 收到 `Notify(summary, body)` 时经会话总线 `org.freedesktop.Notifications`
  弹桌面通知（仅静态文案，如隧道重定向被拒）。
- 无图形会话（无 `WAYLAND_DISPLAY`/`DISPLAY`）时**不注册**。
- agent 掉线（D-Bus 名消失）时，守护进程注销该 agent 并把在途事务按
  取消失败处理。

### 3.3 QR helper `ucable-agent-helper`

- 默认：从 stdin 第一行读取 QR URL、后续读取状态行的一次性窗口进程。
- `--select`：设备选择窗口（"Use phone" / "Cancel"），不读 stdin；退出码
  0 表示用户选择用手机，其余（取消/关闭/超时）表示放弃。

## 3.4 多设备选择（与实体密钥共存）

Firefox 检测到多个认证器时只显示一句"Multiple devices found. Please
select one."，**只能靠触摸实体密钥来选择**——它会把每个设备的
`getInfo` 探测（FIDO2.0 用 dummy `makeCredential`，`rp.id=make.me.blink`）
发给设备，只有回复 `Pin*` 状态的设备才算被选中（见 authenticator-rs
`transport/mod.rs::block_and_blink`）。虚拟设备没有可触摸的按键，因此：

- 守护进程收到 blink 探测时，不再直接回 `0x2C`（会被判为未选中），而是
  让 agent 弹出"Use phone"窗口。
- 用户点"Use phone"→ 回 `0x35`（CTAP2_ERR_PIN_NOT_SET，被判定为选中），
  Firefox 随即发真正的请求，进入常规 QR 流程。
- 用户点"Cancel"/关窗/超时 → 回 `0x2C`（未选中），实体密钥仍可被选中。
- 用户触摸实体密钥 → Firefox 取消我们的 blink（`CTAPHID_CANCEL`），守护
  进程收到 `CancelRelay` 时关掉选择窗口，不给任何回复。
- `--no-ui` 或无 agent 时无法弹窗，保持旧行为（直接回 `0x2C`，即不参与
  选择）。

## 4. D-Bus 接口

守护进程实现（对象 `/org/ucabled/Manager`）：

```
interface org.ucabled.Manager1
  RegisterAgent(o path)            # agent 注册其唯一 bus name 上的对象路径
  UnregisterAgent()
  TransactionCancelled(t tid)         # 用户关闭 QR 窗口
  SelectionResult(t tid, b use_phone) # 用户对设备选择窗口的回答
```

agent 实现（对象路径由 `RegisterAgent` 传入）：

```
interface org.ucabled.Agent1
  Prompt(t tid, s url, s rp, t timeout_secs)
  Select(t tid)                    # 弹设备选择窗口
  Found(t tid)
  Close(t tid)
  Notify(s summary, s body)        # 桌面通知（仅守护进程静态文案）
```

- `tid` 复用 HID channel `cid`，便于与在途事务关联。
- URL（内含 16 字节事务 secret）只经 system bus 的 **unicast** 传给已授权
  agent，再由 stdin 给 helper；不进 argv、不进日志。
- 守护进程回调 agent 时使用注册时记录的唯一 bus name 作为 destination。

## 5. 授权：用 polkit 限定"当前活动本地会话用户"

不自己查 logind，而是把授权委托给 polkit；已核对 polkit 实现：

- polkit 的 D-Bus policy（`data/org.freedesktop.PolicyKit1.conf.in`）对默认
  上下文有 `<allow send_destination="org.freedesktop.PolicyKit1"/>`，**任意
  用户（含非 root 的守护进程）都能调用 `CheckAuthorization`**。
- 主体可用 **`system-bus-name`**（键 `name`，必须是唯一名 `:1.x`）；
  `polkit_subject_new_for_gvariant_invocation()` 对这类主体**没有**
  "调用者与主体必须同 uid"的限制，主体由 bus 保证不可伪造。
- action 的 `allow_active` 语义是"**本地控制台上的活动用户会话**"，因此
  SSH/远程会话天然不满足。

流程：

1. 收到 `RegisterAgent`，取消息头的 sender **唯一名** `:1.x`。
2. 调 `CheckAuthorization(subject=("system-bus-name", {name: ":1.x"}),
   action_id="org.ucabled.register-agent", details={}, flags=0,
   cancellation_id="")`。
3. 仅当 `is_authorized` 才登记该 agent（记录唯一名、对象路径、uid）。
4. 每次 `Prompt` 前**重新复核**一次授权（应对快速用户切换），选择当前
   仍被授权的 agent；不在引用计数上做任何信任。

发布一个 polkit action：

```xml
<action id="org.ucabled.register-agent">
  <description>Register the passkey dialog agent</description>
  <message>Authentication is required to show passkey prompts</message>
  <defaults>
    <allow_any>no</allow_any>
    <allow_inactive>no</allow_inactive>
    <allow_active>yes</allow_active>
  </defaults>
  <annotate key="org.freedesktop.policykit.owner">unix-user:ucabled</annotate>
</action>
```

> **`org.freedesktop.policykit.owner` 注解是必需的**：非 root 的 `ucabled`
> 守护进程对**其他 uid**（会话用户）的 subject 调 `CheckAuthorization` 时，
> polkit 默认只放行 uid 0 或 action 的 owner；没有这个注解，跨 uid 检查
> 会被直接拒绝（"polkit check fails"）。另外 NixOS 的 polkit 用
> `--datadir=/run/current-system/sw/share` 构建，因此 action 文件随包放进
> `share/polkit-1/actions/` 并加入 `environment.systemPackages` 即可被发现。

需要更严时可再加 JS 规则，把"本地"显式化：

```javascript
polkit.addRule(function (action, subject) {
  if (action.id === "org.ucabled.register-agent" &&
      subject.local && subject.active) {
    return polkit.Result.YES;
  }
});
```

这样**只有当前活动本地会话的用户**能注册，且为静默授权、不弹窗。

## 6. 安全属性

- **M2 解决**：只有 `ucabled` 服务账号（经 `/dev/uhid` 上的 ACL）能打开该
  节点；人类用户的进程无法创建任何虚拟 HID 设备。Firefox 不受影响（走
  hidraw uaccess）。
- **M3 强化**：守护进程在真正的 system unit sandbox 中运行，且因为没有
  GUI 可以更严：`MemoryDenyWriteExecute`、`SystemCallFilter=@system-service`、
  空 `CapabilityBoundingSet`、`RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
  AF_BLUETOOTH AF_NETLINK`，加上文件系统/命名空间保护。（`DevicePolicy`/
  `DeviceAllow` 因路径解析有失败模式且与 ACL 冗余，故未启用。）
- QR secret 仅经 system bus unicast → agent → helper stdin，不进 argv/日志。
- agent 只能来自活动本地会话用户，且在 `Prompt` 时复核。

## 7. 生命周期与边界情况

- **boot 常驻**（暂定）：设备在登录前即存在；无 agent 时事务快速失败。
- **锁屏**：session 仍 `Active`，但窗口不可交互 → 事务超时；可接受。
- **快速用户切换**：登记时校验 + `Prompt` 时复核，始终发往当前活动的
  被授权 agent。
- **SSH/远程**：`allow_active` 不含远程，天然排除。
- **本地 TTY**：`allow_active` 可能放行，但我们不提供 tty agent，因此
  不会有 agent 注册（tty agent 留待以后）。
- **agent 掉线**：注销 agent；在途事务按取消处理。
- **未安装/未运行 polkitd**：授权失败 → 无 agent，事务失败；桌面环境
  默认有 polkit，可接受。

## 8. 打包与配置

NixOS module（`nix/module.nix`）：

- `users.users.ucabled` / `users.groups.ucabled`（`isSystemUser`）。
- udev：`KERNEL=="uhid", RUN+="... setfacl -m u:ucabled:rw /dev/uhid"`（ACL 授予，
  不改节点组；取代 `uaccess`）。
- `systemd.services.ucabled`（system unit，含第 6 节的 sandbox）。
- `systemd.user.services.ucable-agent`：作为全局 user unit 对所有用户启用
  （`wantedBy = graphical-session.target`，`PartOf`/`After` 同一 target）；
  不必按用户配置，能否弹窗由 polkit 判定。**不绑 `default.target`**：开了
  linger 时后者会在无人登录时启动，此时 `DISPLAY`/`WAYLAND_DISPLAY` 尚未
  导入；绑到 `graphical-session.target` 则随图形会话启动/停止，环境已就绪。
- system D-Bus policy（`org.ucabled`）：允许 `ucabled` 拥有该名字并回调
  agent，允许本机用户发往 `org.ucabled`。
- polkit action（第 5 节）+ 可选 JS 规则。
- BlueZ 的 polkit 规则（让非会话用户的守护进程能控制适配器）。
- `hardware.bluetooth.enable`、`boot.kernelModules=[uhid]` 维持。

其它：

- `dist/`：系统 unit + 用户 agent unit + 更新后的 udev 规则。
- `nix/package.nix`：发布 `ucabled`、`ucable-agent`、`ucable-agent-helper`。
- 依赖：直接依赖 `dbus` + `dbus-crossroads` + `dbus-tokio`（与 `bluer`
  同一套，不引入 zbus）。

## 9. 与旧模型对比与迁移

- 删除/停用旧的 `~/.config/systemd/user/ucabled.service`；改用 system unit。
- `systemctl --user stop` 不再是 NFR-3 的"开关"；改为启停 system unit（可
  另行配置 polkit，允许活动会话用户启停，暂不做）。
- README 的安装/安全/排障章节需按新模型重写。

## 10. 明确不做 / 否决的方案

- 否决 **abstract unix socket + peer cred** 自研协议：D-Bus + polkit 更标准、
  有 policy 与现成会话判定。
- 不做按用户配置的鉴权后备：授权完全交给 polkit，polkitd 不可用时事务
  直接失败。
- 不做 **tty/SSH agent**（不在当前范围）。

## 11. 已定 / 仍待处理

已定：

- 命名：`org.ucabled` + `org.ucabled.Manager1` / `org.ucabled.Agent1`。
- D-Bus 库：**dbus / dbus-crossroads / dbus-tokio**（与 `bluer` 同一套依赖，
  避免新增 D-Bus 实现；daemon 与 agent 各用一条异步连接，由一个路由 task
  服务 crossroads，出站调用走 nonblock proxy）。
- 守护进程启动时机：system unit，boot 常驻（`multi-user.target`）。
- polkit action `org.ucabled.register-agent`，`allow_active=yes`。

仍待处理：

- 活动会话用户启停 system unit：仍未开放，当前用 `systemctl start/stop
  ucabled`（需提权）。NFR-3 已由 agent 的"Use phone"选择窗口解决（见 §3.4），
  不再需要该开关。

## 12. 实现与提交计划

1. ~~引入 D-Bus agent 层~~：`src/agent.rs`（`Manager1` 服务 + `Agent1` 回调 +
   注册表 + polkit 授权）。
2. ~~新增 `ucable-agent` agent~~：实现 `Agent1`，复用 `ucable-agent-helper`。
3. ~~守护进程改用 D-Bus UI 驱动~~；无 agent 快速失败；移除进程内 GUI。
4. ~~系统服务化~~：system 用户/组、udev ACL 规则、system unit + sandbox、
   agent unit、D-Bus policy、polkit action（+ 规则）、BlueZ polkit、dist
   单元。
5. ~~文档~~：README 与 `docs/plan.md`。

## 13. 参考

- polkit `org.freedesktop.PolicyKit1` D-Bus API；`data/org.freedesktop.
  PolicyKit1.conf.in`（默认上下文允许向 polkit 发送）；`src/polkit/
  polkitsubject.c`（`system-bus-name` 主体解析）；GeoClue 的非 root agent
  鉴权模式。
- FIDO CTAP 2.2 §11.5（Hybrid transports / caBLE v2）。
- systemd `systemd.exec(5)` sandbox 指令（`DevicePolicy`/`DeviceAllow` 等）。
