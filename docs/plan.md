# Phone Passkey Bridge — 需求与实现计划（修订版）

> 在 Linux 桌面（Firefox，零改动、零扩展）上复刻 Chromium 的跨设备 passkey 流程：
> 通过 QR 码把手机（iCloud Keychain / Google Password Manager）变为 passkey 的
> 唯一存储位置，笔记本仅承担 QR 展示与通信中继。
>
> 本版已按评审意见修订：QR 编码与结构、握手密钥派生、BLE 方向、HID report
> descriptor、CBOR 最小解析、里程碑排序（spike 前置）。

## 1. 背景与动机

- Firefox 桌面版在 Linux 上只支持基于 USB-HID 的硬件安全密钥（YubiKey 等），
没有平台 authenticator，也没有 Chromium 的 caBLE v2（QR + BLE）跨设备流程。
- KeePassXC 类方案（扩展注入 + 本机存储）把密钥留在本机；Chromium 的 hybrid
流程则把密钥留在手机。本项目采用后者模型。
- 实现路线：**虚拟 HID FIDO2 设备**（`/dev/uhid`）+ 本机守护进程做 caBLE v2
initiator。不需要浏览器扩展，不需要打补丁，不需要 native messaging。

## 2. 需求

### 2.1 功能需求

| 编号 | 需求 | 优先级 |
| --- | --- | --- |
| FR-1 | 向浏览器呈现符合 U2FHID/CTAP2 规范的虚拟 FIDO2 设备（via `/dev/uhid`），Firefox 零改动识别为安全密钥 | P0 |
| FR-2 | `authenticatorMakeCredential` / `authenticatorGetAssertion` 经 caBLE v2 中继至手机；本机不做 FIDO 加密学、不存储密钥 | P0 |
| FR-3 | QR 码展示：MVP 终端 Unicode 方块码，正式版 egui 浮窗（RP 域名 + 取消按钮） | P0 / P1 |
| FR-4 | 长操作期间发送 KEEPALIVE（STATUS_UPNEEDED）；正确处理 CANCEL、超时、断连，错误码正确映射回 Firefox | P0 |
| FR-5 | getInfo 能力协商：`versions: ["FIDO_2_0"]`，`options: {rk, up, uv}`，`clientPin: false`，`maxMsgSize: 7609`，`transports: ["hybrid"]`，不广告 U2F_V2 | P0 |
| FR-6 | 支持 iPhone（iCloud Keychain）与 Android（Google Password Manager）扫码；手机端零安装 | P0 |
| FR-7 | state-assisted linking（"记住这台手机"）：存储 contact ID，后续操作免扫码 | P2 |
| FR-8a | **BLE advert 接收**：扫描手机广播的 EID（UUID 0000fff9 的 20 字节 service data），trial-decrypt 得到 routing ID 与连接 nonce。**协议必需**（CTAP 2.2 §11.5.1：PSK 由 QR secret + 解密后的 BLE advert 派生，作为 proximity proof；无 BLE 则无握手） | P0 |
| FR-8b | BLE GATT 数据通道（桌面作 central 连接手机的 GATT server）作为隧道的替代传输。Chromium/iOS/Android 实际均走隧道，预期不需要 | P3 |
| FR-9 | per-RP / 全局策略配置（何时走手机流程） | P2 |

### 2.2 非功能需求

| 编号 | 需求 |
| --- | --- |
| NFR-1 | 本机零密钥材料；隧道流量端到端加密（AES-256-GCM，密钥由 X25519 ECDH + QR secret 经 HKDF 派生），隧道服务器不可见明文 |
| NFR-2 | 常驻进程资源占用小（systemd user service，事件驱动） |
| NFR-3 | 与真实硬件密钥共存行为可预期；提供 daemon 开关作为缓解 |
| NFR-4 | NixOS 可打包（flake），附 udev 规则与 uhid 内核模块配置 |
| NFR-5 | 纯 Rust 实现；BLE 使用 bluer（BlueZ D-Bus） |
| NFR-6 | 安全默认值：pairing 状态文件权限 0600；日志只记命令字与长度，不落 CBOR payload 原文（含 clientDataHash、rpId 等敏感上下文） |
| NFR-7 | 文档化 `/dev/uhid` 的权限含义：`uaccess` 授予 seat 用户的**任意**进程创建虚拟 HID 设备（含键盘/输入注入）的能力，并非仅限本设备的 FIDO 用途；多用户或不可信进程环境下应改用专用 group 收窄到指定账户（见 README Security） |

### 2.3 非目标（Non-goals）

- 本机 passkey 存储（softoken）——明确不做，密钥只在手机。
- 系统级 D-Bus portal / 为 Chromium 做适配（架构上兼容，但不作为目标）。
- U2F APDU 层（不广告 `U2F_V2`，收到 MSG 命令直接报错）。
- 本地实现 PIN/UV 协议——全部由手机完成，本机只透传。

## 3. 术语

- **caBLE v2 (Cloud Assisted BLE)**：FIDO CTAP 2.2 §11.5 定义的跨设备传输；桌面端
显示 QR（内含 ephemeral 公钥、QR secret、隧道服务器域名），手机扫码后经
隧道服务器 WSS 与桌面端交换 CTAP2，可选切换到 BLE。
- **U2FHID / CTAP-HID**：FIDO2 设备 over USB-HID 的帧协议（INIT/CONT 分片，
最大 payload 7609 字节 @ 64 字节 report）。
- **CTAP2**：authenticator 协议（CBOR 命令：MakeCredential 0x01、GetAssertion
0x02、GetInfo 0x04 等）。
- **uhid**：Linux 内核 userspace HID 驱动，可在用户态注册虚拟 HID 设备。

## 4. 总体架构

```
Firefox ──CTAP-HID (64B reports, usage page 0xF1D0)──▶ /dev/uhid ──▶ phone-passkey-d
                                                                      (Rust daemon)
                                                                        ├ uhid device
                                                                        ├ U2FHID transport
                                                                        ├ CTAP2 relay
                                                                        ├ caBLE v2 initiator
                                                                        └ QR UI (egui)
                                                                              │
                       BLE advert (EID, proximity proof) + WSS tunnel (CTAP 数据通道)
                                                                              │
                                                                   iPhone / Android
                                                                   (iCloud KC / GPM)
```

**核心决策：中继（dumb pipe）模型。** daemon 不做任何 FIDO 加密学、不存储
密钥，`authenticatorMakeCredential`/`GetAssertion` 的请求与响应在手机与
Firefox 之间搬运。唯一例外：为在 QR 浮窗显示 RP 域名（FR-3），对请求 CBOR
做**最小只读解析**，仅提取 `rp.id` / `rpId` 字段，不校验、不修改、不缓存。

由此获得：

- 本机零密钥材料，审计面最小；
- 扩展天然透传（hmac-secret、prf、credProtect 等输入本就在 CBOR 内）；
- attestation 是手机的真实 attestation（Apple/Google），无自签信任问题；
- uv=true 声明诚实（生物识别在手机完成，UV 位由手机响应带回）。

## 5. 关键设计

### 5.1 CTAP2 命令策略

| 命令 | 处理 |
| --- | --- |
| GetInfo (0x04) | 本地应答：见 FR-5；aaguid 为随机固定值；extensions 静态列表（先保守，实测后加 hmac-secret/prf） |
| MakeCredential (0x01) / GetAssertion (0x02) | 启动 caBLE 流程，CBOR 透传（仅只读提取 rpId 用于 UI）；等待期间持续发 KEEPALIVE |
| ClientPIN / credMgmt / Reset / 其他 | 返回 COMMAND_NOT_SUPPORTED / INVALID_COMMAND |
| CANCEL (U2FHID 0x91) | 终止隧道，挂起的 CBOR 回 `0x2D` (KEEPALIVE_CANCEL) |
| 手机无凭证 / 用户取消 | `0x2E` (NO_CREDENTIALS) / `0x2D` |

### 5.2 caBLE v2 initiator 流程

**以实现为准的参考源是 Chromium `device/fido/cable`**（`qr_generator.cc`、
`tunnel_server_client`、v2 handshake 相关文件），CTAP 2.2 §11.5 作为交叉校验；
这一层是项目最脆的部分，不凭 spec 摘要臆写。要点：

1. 生成随机 **P-256** identity 密钥对与 16 字节 QR secret（spec 允许随机；
Chromium 从 32 字节 seed 确定性派生以便日后验证 pairing 签名，FR-7 阶段
再改 seed 派生）。
2. QR payload = canonical CBOR map：`{0: 33 字节压缩公钥, 1: 16 字节 secret,
2: 已知隧道域名数(=2), 3: epoch 秒, 4: supports_linking, 5: "mc"/"ga"}`。
QR 内容为 `FIDO:/` + digitEncode(CBOR)：每 7 字节小端转 u64，补零成
17 位十进制；尾部按 1..6 字节用 3/5/8/10/13/15 位（QR numeric mode；
**不是 base64，也不是原始二进制**）。
3. **BLE advert 接收（协议必需）**：eidKey = HKDF(secret, salt=空, info=LE32(1))
导出 64 字节（32 AES + 32 HMAC）。扫描 UUID `0000fff9-0000-1000-8000-
00805f9b34fb` 的 20 字节 service data：HMAC-SHA256 前 4 字节校验 +
AES-256 单块解密，得 16 字节 plaintext EID = `[0x00][nonce 10B]
[routing ID 3B][隧道域名 u16 LE]`。
4. 派生：tunnelID = HKDF(secret, salt=空, info=LE32(2))；
**PSK = HKDF(secret, salt=plaintext EID, info=LE32(3))**——BLE advert 由此
成为 proximity proof，密码学上不可绕过。
5. WSS 连接 `wss://<domain>/cable/connect/<hex routingID>/<hex tunnelID>`，
子协议 `fido.cable`，跟随重定向。手机侧走 `/cable/new/<hex tunnelID>`，
隧道服务器在响应头 `X-caBLE-Routing-Id` 分配 routing ID。
6. 握手 = **Noise KNpsk0 (P-256/AES-GCM/SHA-256)**，桌面为 initiator：
prologue `0x01` → MixHash(identity 公钥未压缩 65B) → MixKeyAndHash(PSK)
→ MixHash+MixKey(ephemeral 公钥) → 空明文加密。初始消息 81 字节
(65+16)，响应同为 81 字节；响应处理混入 ee 与 se 两个 ECDH 结果。
traffic keys = HKDF2(ck, 空)，initiator 取 (write, read)。
7. 传输加密：AES-256-GCM，nonce = 8 零字节 ‖ BE32 计数器（按方向），
AD 为空；明文先补零至 32 字节倍数，末字节记补零数。消息首字节为类型
（0=shutdown, 1=CTAP, 2=update），**类型字节在 AEAD 之内**。手机握手后
首条消息为 post-handshake（无类型字节的 CBOR map `{1: getInfo 响应}`）。
8. 手机 CBOR 响应 → 封装进 U2FHID CBOR 响应 → Firefox。
9. 手机断连/超时 → 按 5.1 映射错误码。

### 5.3 Firefox 侧注意点

- **设备常驻**：浏览器在 WebAuthn 调用时才枚举 hidraw，daemon 必须为 systemd
user service 长驻进程。
- **KEEPALIVE 是硬需求**：扫码可能耗时 30s+，不发 STATUS_UPNEEDED（每
100–300ms）Firefox 会判超时；总操作时限 5 分钟。
- **多通道**：繁忙时收到 broadcast INIT 回 ERR_CHANNEL_BUSY。
- **多设备共存**：与真实 YubiKey 同时存在时 Firefox 的行为需实测；缓解手段是
daemon 开关（需要时才注册虚拟设备），可用快捷键
`systemctl --user start/stop phone-passkey-d`。
- 沙箱 Firefox（Flatpak/Snap）需额外 udev 规则让沙箱看到 hidraw；NixOS 原生
包无此问题。
- **credential transports 恒为 `usb`**：RP 看到的 `transports` 由 Firefox 决定，
不是设备能控制的。Linux 的 CTAP 后端（`dom/webauthn/authrs_bridge/src/lib.rs`
的 `get_transports`）对所有走 USB-HID 的凭证硬编码返回 `["usb"]`，只在
softtoken + platform attachment 时返回 `"internal"`。因此即使我们在
`getInfo` 里广告 `transports: ["hybrid"]`，webauthn.io 等 RP 仍显示 `usb`
（凭证本身仍被识别为 synced passkey / iCloud Keychain，因为那来自 attestation
的 AAGUID 与 backup 标志）。纯属标签问题，不影响注册与登录；仅影响 RP 对
transport 的 UI 判断，以及后续 getAssertion 可能带回的 `["usb"]` hint。

### 5.4 模块与选型

| 模块 | crate | 说明 |
| --- | --- | --- |
| uhid 设备 | 手写（`src/uhid_dev.rs`） | 直接读写 `/dev/uhid`（tokio `AsyncFd` + `libc`），不用维护状态一般的 `uhid-virt`；FIDO 标准 report descriptor（U2FHID spec §4.3，34 字节）：`06 D0 F1 09 01 A1 01 09 20 15 00 26 FF 00 75 08 95 40 81 02 09 21 15 00 26 FF 00 75 08 95 40 91 02 C0`（Usage Page 0xF1D0 是 16 位值，必须用长项 `06 D0 F1`） |
| 运行时 | `tokio` | HID / 隧道 / UI 共用一个 runtime |
| U2FHID 传输层 | 手写 | INIT/PING/WINK/CANCEL/CBOR/ERROR/KEEPALIVE；分片重组；最大 7609B |
| CBOR | `ciborium` | getInfo 构造、命令分发、rpId 最小只读提取；其余业务 CBOR 不解析 |
| caBLE v2 | 手写 | 逐函数对齐 Chromium `device/fido/cable`；无成熟独立移植，边界清晰 |
| 隧道 | `tokio-tungstenite` + `rustls` | WSS |
| 加密 | `p256`(ECDH) + `hkdf` + `sha2` + `hmac` + `aes` + `aes-gcm` | caBLE v2 握手（Noise P-256）与消息加密 |
| QR | `qrcode` | payload 按十进制数字串编码（numeric mode），终端 Unicode 方块码 / egui 窗口 |
| UI | `eframe`/`egui`（无边框置顶窗，per-transaction 子进程 `ucabled-qr`；Wayland 无法隐藏窗口，故不用常驻窗口；缺失或 `--no-ui` 时回落终端 QR） | RP 域名（窗口标题） + QR + 取消（关窗） |
| BLE | `bluer`(feature `bluetoothd`) | BlueZ discovery 扫描手机 EID advert（FR-8a，协议必需）；GATT central 预期不需要 |

代码量预估：spike（QR + 隧道握手 + 假 CBOR 往返）500–800 行；传输层
700–1000 行；CTAP2 分发 200–400 行；caBLE initiator 完整化 1000–1500 行；
UI/胶水 300–500 行。MVP 约 2500–3500 行。

## 6. 实现计划

### S0 — 可行性 Spike（验收：真机扫码完成握手与一次 getInfo 往返）

caBLE 层与 uhid/HID 层完全解耦，**先于一切验证**，避免在错误前提上投入
M0–M2。

- [x] QR 生成（P-256 identity + secret，§5.2 第 2 步）+ 终端渲染
- [x] WSS 隧道 + Noise KNpsk0 握手 + 消息加解密（逐函数对齐 Chromium
`device/fido/cable`）
- [x] BLE advert 扫描与 EID trial-decrypt（bluer，FR-8a）
- [x] 自写 mock phone 对拍：本地 relay 上完成全往返（`cargo test`）
- [x] **真机验收**：iPhone 扫码完成握手与一次 getAssertion 往返
（2026-09-29 通过：iOS 正确解析请求并显示"无该网站通行密钥"；
注意 caBLE CTAP 消息 = 类型字节 + 完整 CTAP2 命令[含命令字节]；
iOS 不接受 hybrid 上的裸 getInfo；iOS 用户取消时直接断隧道不回错误）
- [ ] Android 真机复测（协议相同，预期通过，低风险）

### M0 — 虚拟设备打通（验收：Firefox 能枚举）

- [x] uhid 设备注册（上述 34 字节 descriptor），命名如 "Phone Passkey Bridge"
- [x] U2FHID INIT / PING / WINK / ERROR 帧处理
- [x] systemd user service + udev/uaccess 规则
- [x] 验收：`ls /dev/hidraw*` 可见；webauthn.io 探测到 security key

### M1 — CTAP2 应答（验收：webauthn.io 触发 makeCredential 并挂起）

- [x] CBOR 命令分发；GetInfo 按 FR-5 应答
- [x] MakeCredential/GetAssertion：进入 KEEPALIVE 循环（150ms UPNEEDED）
- [x] CANCEL 正确应答挂起的 CBOR（0x2D KEEPALIVE_CANCEL）

### M2 — caBLE initiator 完整化（验收：隧道握手成功，模拟 responder 全往返）

- [x] S0 spike 代码产品化：QR 生成、WSS 隧道、握手、消息加解密
- [x] 模拟 responder 完成一次完整 CTAP 往返（集成测试）

### M3 — 真机端到端（验收：iPhone + Android 各完成注册与登录）

- [x] CBOR 中继接线；错误码映射；CANCEL/超时路径
- [x] iPhone 真机验收：webauthn.io 注册 + 登录成功（2026-09-29）
- [x] Firefox 兼容处理：iOS 要求 rp.name/user.displayName（最小注入）；
      Firefox `up=false` 静默探测（本地伪造成功响应）与
      "make.me.blink" 闪灯探测（本地回错误，不转发）
- [ ] Android 真机复测

### M4 — UX 打磨（验收：日常使用可接受）

- [x] QR 浮窗：RP 域名、QR、取消（egui/wgpu 无边框置顶窗，per-transaction
      子进程 `ucabled-qr`；Wayland 无法隐藏窗口，故不用常驻窗口）
- [x] 取消/超时/断连路径；日志
- [ ] 与真实 YubiKey 共存实测；daemon 开关
- [x] NixOS flake 打包 + systemd user service（module 见 §8）
- [x] systemd 常驻实测通过（2026-09-29：登录自启、GUI 弹窗正常）

### M5 — 增强（可选）

- [ ] FR-8b（预期不需要）：BLE GATT 数据通道（桌面 central）
- [ ] ~~FR-7：state-assisted "remember this phone"~~ **搁置**（2026-09-29 真机验证：iOS 不支持 linking；无 Android 设备复测）——设计见 docs/linking.md
- [ ] FR-9：per-RP 策略
- [ ] extensions 静态列表按真机实测结果扩充

## 7. 风险清单（按优先级）

1. ~~隧道-only 是否被手机接受~~——**已解决**：spec 考证确认 BLE advert 是
QR 流程的协议必需项（PSK 绑定解密后的 advert，proximity proof），
隧道-only 在密码学上不可能。方案已调整为 BLE 扫描（FR-8a，仅接收、
无需 GATT）+ 隧道传 CTAP。真机兼容性仍待 S0 真机验收最终确认。
2. Firefox 多 FIDO 设备枚举行为——实测 + daemon 开关缓解。
3. getInfo extensions 静态广告与真机能力偏差——表现为个别 RP 报
INVALID_OPTION，可接受、可迭代。
4. 隧道服务器可用性/合规——缓释：协议允许 QR 内换自建域名。

## 8. NixOS 配置

仓库自带 flake：包 `ucabled`（rustPlatform.buildRustPackage，只发布
`ucabled` + `ucabled-qr`，QR helper 包装了 Vulkan/Wayland 运行时库路径），
NixOS module `nixosModules.ucabled`，devShell。

```nix
{
  # configuration.nix / flake 引用：
  imports = [ inputs.ucabled.nixosModules.ucabled ];
  services.ucabled.enable = true;
  # 模块自动带上：boot.kernelModules=[uhid]、udev uaccess 规则、
  # hardware.bluetooth.enable=mkDefault true、systemd user service
}
```

手工等效配置（不使用 module 时）：

```nix
{
  boot.kernelModules = [ "uhid" ];
  services.udev.extraRules = ''KERNEL=="uhid", TAG+="uaccess"'';
  hardware.bluetooth.enable = true;
}
```

## 9. 参考

- FIDO CTAP 2.2 spec §11.5（Hybrid transports / caBLE v2）
- Chromium `device/fido/cable`（实现对齐的第一参考源：`qr_generator.cc`、
`tunnel_server_client`、v2 handshake）
- 虚拟设备先例：token2-fido-bridge、tpm-fido、virtual-fido、soft-fido2(passless)
- 同生态项目：linux-credentials（credentialsd / libwebauthn）
