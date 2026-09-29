# 系统服务与用户 UI Agent 设计（M2/M3 修订）

> 状态：已实现。代码见 `src/prompter.rs`（D-Bus 层与 polkit 授权）、
> `src/bin/ucabled-ui.rs`（session agent）、`src/bin/ucabled.rs`（daemon
> 接线）与 `nix/module.nix` / `dist/`（打包）。
> 本文取代 `docs/plan.md` 中"systemd user service"的运行模型，以及 M2/M3
> 两次临时缓解（udev `uaccess` + user unit sandbox）。
>
> 目标：守护进程以**独立系统用户**运行并独占 `/dev/uhid`；UI 由每个用户
> 会话里的 agent 通过 **system D-Bus** 提供；用 **polkit** 保证只有**当前
> 活动本地会话的用户**能注册 prompter。

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
Firefox ──hidraw──▶ [virtual FIDO2] ──/dev/uhid──▶ ucabled.service   (system, User=ucabled)
                                                    HID + U2FHID + caBLE (BLE + WSS)
                                                    D-Bus: 拥有 org.ucabled，实现 Ui1
                                                          ▲ RegisterPrompter()   │ Prompt/Found/Close
                                                          │ TransactionCancelled()│ (回调)
                                                    ucabled-ui.service (user, session)
                                                    实现 org.ucabled.Prompter1
                                                          └─ spawn ucabled-qr（Wayland/wgpu 窗口）
```

`ucabled-qr`、`DesktopFlow`、Noise/隧道、BLE 扫描等既有实现不变；
`run_qr_transaction` 的 closure 接口保持，只是 `on_qr`/`on_advert`/`hide`
从"进程内 spawn GUI"变成"经 D-Bus 通知 agent"。

## 3. 组件

### 3.1 系统守护进程 `ucabled.service`

- system unit，`User=ucabled`、`Group=ucabled`（专用系统账号）。
- 依赖：`/dev/uhid`（udev ACL `setfacl -m u:ucabled:rw`，不改节点组）、BlueZ（system
  bus）、网络（WSS 隧道）。
- 拥有 system bus 名 `org.ucabled`（暂定），对象 `/org/ucabled/Ui`，实现
  接口 `org.ucabled.Ui1`。
- 仍然负责 HID 收发、U2FHID 分片、CTAP 分发、caBLE 握手与中继；**不再
  spawn 任何 GUI 子进程**。
- 无可用 prompter 时，对 `MakeCredential`/`GetAssertion` 直接回 CTAP 错误
  并记日志，不启动一个无人可见的 QR 事务。

### 3.2 用户 agent `ucabled-ui.service`

- user unit，对所有用户启用；谁真正能弹窗完全由 polkit 决定，不按用户配置。
- 连接到 **system bus**，导出一个对象实现 `org.ucabled.Prompter1`，调用
  `org.ucabled.Ui1.RegisterPrompter(path)` 注册自己。
- 收到 `Prompt(tid, url, rp, timeout)` 时，按现有 `GuiHandle` 逻辑 spawn
  `ucabled-qr`，把 URL 写进它的 stdin；把 helper 的退出转成
  `TransactionCancelled(tid)`；收到 `Found`/`Close` 时更新/关闭窗口。
- 无图形会话（无 `WAYLAND_DISPLAY`/`DISPLAY`）时**不注册**。
- agent 掉线（D-Bus 名消失）时，守护进程注销该 prompter 并把在途事务按
  取消失败处理。

### 3.3 QR helper `ucabled-qr`（不变）

仍是从 stdin 第一行读取 QR URL、后续读取状态行的一次性窗口进程。

## 4. D-Bus 接口

守护进程实现（对象 `/org/ucabled/Ui`）：

```
interface org.ucabled.Ui1
  RegisterPrompter(o path)            # agent 注册其唯一 bus name 上的对象路径
  UnregisterPrompter()
  TransactionCancelled(t tid)         # 用户关闭窗口
```

agent 实现（对象路径由 `RegisterPrompter` 传入）：

```
interface org.ucabled.Prompter1
  Prompt(t tid, s url, s rp, t timeout_secs)
  Found(t tid)
  Close(t tid)
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

1. 收到 `RegisterPrompter`，取消息头的 sender **唯一名** `:1.x`。
2. 调 `CheckAuthorization(subject=("system-bus-name", {name: ":1.x"}),
   action_id="org.ucabled.register-prompter", details={}, flags=0,
   cancellation_id="")`。
3. 仅当 `is_authorized` 才登记该 prompter（记录唯一名、对象路径、uid）。
4. 每次 `Prompt` 前**重新复核**一次授权（应对快速用户切换），选择当前
   仍被授权的 prompter；不在引用计数上做任何信任。

发布一个 polkit action：

```xml
<action id="org.ucabled.register-prompter">
  <description>Register the passkey dialog prompter</description>
  <message>Authentication is required to show passkey prompts</message>
  <defaults>
    <allow_any>no</allow_any>
    <allow_inactive>no</allow_inactive>
    <allow_active>yes</allow_active>
  </defaults>
</action>
```

需要更严时可再加 JS 规则，把"本地"显式化：

```javascript
polkit.addRule(function (action, subject) {
  if (action.id === "org.ucabled.register-prompter" &&
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
- prompter 只能来自活动本地会话用户，且在 `Prompt` 时复核。

## 7. 生命周期与边界情况

- **boot 常驻**（暂定）：设备在登录前即存在；无 prompter 时事务快速失败。
- **锁屏**：session 仍 `Active`，但窗口不可交互 → 事务超时；可接受。
- **快速用户切换**：登记时校验 + `Prompt` 时复核，始终发往当前活动的
  被授权 prompter。
- **SSH/远程**：`allow_active` 不含远程，天然排除。
- **本地 TTY**：`allow_active` 可能放行，但我们不提供 tty prompter，因此
  不会有 agent 注册（tty prompter 留待以后）。
- **agent 掉线**：注销 prompter；在途事务按取消处理。
- **未安装/未运行 polkitd**：授权失败 → 无 prompter，事务失败；桌面环境
  默认有 polkit，可接受。

## 8. 打包与配置

NixOS module（`nix/module.nix`）：

- `users.users.ucabled` / `users.groups.ucabled`（`isSystemUser`）。
- udev：`KERNEL=="uhid", RUN+="... setfacl -m u:ucabled:rw /dev/uhid"`（ACL 授予，
  不改节点组；取代 `uaccess`）。
- `systemd.services.ucabled`（system unit，含第 6 节的 sandbox）。
- `systemd.user.services.ucabled-ui`：作为全局 user unit 对所有用户启用
  （`wantedBy = default.target`）；不必按用户配置，能否弹窗由 polkit 判定。
- system D-Bus policy（`org.ucabled`）：允许 `ucabled` 拥有该名字并回调
  agent，允许本机用户发往 `org.ucabled`。
- polkit action（第 5 节）+ 可选 JS 规则。
- BlueZ 的 polkit 规则（让非会话用户的守护进程能控制适配器）。
- `hardware.bluetooth.enable`、`boot.kernelModules=[uhid]` 维持。

其它：

- `dist/`：系统 unit + 用户 agent unit + 更新后的 udev 规则。
- `nix/package.nix`：发布 `ucabled`、`ucabled-ui`、`ucabled-qr`。
- 依赖：直接依赖 `zbus`（与 `bluer` 同大版本，避免重复版本）。

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
- 不做 **tty/SSH prompter**（不在当前范围）。

## 11. 已定 / 仍待处理

已定：

- 命名：`org.ucabled` + `org.ucabled.Ui1` / `org.ucabled.Prompter1`。
- D-Bus 库：**dbus / dbus-crossroads**（与 `bluer` 同一套依赖，避免新增
  依赖；服务与分发各用一个阻塞连接线程）。
- 守护进程启动时机：system unit，boot 常驻（`multi-user.target`）。
- polkit action `org.ucabled.register-prompter`，`allow_active=yes`。

仍待处理：

- 是否允许活动会话用户启停 system unit（NFR-3 开关）：暂未实现，当前用
  `systemctl start/stop ucabled`（需提权）。
- 与真实 YubiKey 共存的行为（NFR-3）仍需实测。
- 真机上 BlueZ 的 polkit/D-Bus 行为需确认（已带 BlueZ polkit 规则兜底）。

## 12. 实现与提交计划

1. ~~引入 D-Bus UI 层~~：`src/prompter.rs`（`Ui1` 服务 + `Prompter1` 回调 +
   注册表 + polkit 授权）。
2. ~~新增 `ucabled-ui` agent~~：实现 `Prompter1`，复用 `ucabled-qr`。
3. ~~守护进程改用 D-Bus UI 驱动~~；无 prompter 快速失败；移除进程内 GUI。
4. ~~系统服务化~~：system 用户/组、udev ACL 规则、system unit + sandbox、
   agent unit、D-Bus policy、polkit action（+ 规则）、BlueZ polkit、dist
   单元。
5. ~~文档~~：README 与 `docs/plan.md`。

## 13. 参考

- polkit `org.freedesktop.PolicyKit1` D-Bus API；`data/org.freedesktop.
  PolicyKit1.conf.in`（默认上下文允许向 polkit 发送）；`src/polkit/
  polkitsubject.c`（`system-bus-name` 主体解析）；GeoClue 的非 root prompter
  鉴权模式。
- FIDO CTAP 2.2 §11.5（Hybrid transports / caBLE v2）。
- systemd `systemd.exec(5)` sandbox 指令（`DevicePolicy`/`DeviceAllow` 等）。
