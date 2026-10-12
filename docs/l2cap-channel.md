# BLE L2CAP 数据通道（CTAP 2.3 hybrid）

> 状态：桌面侧已实现（实验性，Cargo feature `l2cap`，运行时
> `UCABLED_BLE_CHANNEL=1`，默认仍走 WebSocket）。**iPhone 实测（2026-10-02
> 初测、2026-10-13 严格复测，iOS 27.0）不支持该通道**：联网时 advert 只有 20 字节、
> 无 suffix/PSM；飞行模式（仅蓝牙）下 iOS 必须先连隧道拿 routing ID 才能
> 广播，离线时连广播都发不出，手机转圈后报"操作无法完成"——它设计场景
> （离线）下完全无法开始流程。故该特性目前只对 Android 有意义；iOS 若日后
> 支持则无需改协议。

## 1. 结论

1. CTAP 2.3 给 hybrid（§11.5）新增了 **BLE 数据通道**，走 insecure L2CAP
   CoC，把 CTAP 流量完全从隧道服务器上拿走。手机可从双方支持集的交集中
   并行尝试多个通道，任一通道建立后放弃其余（不支持并发传输）；
   WebSocket 是双方都应支持的回退通道。
2. 它与 §11.4 的 FIDO GATT service（`0xFFFD`）**无关**；GATT 与本项目手机的
   关系见 `docs/gatt-data-channel.md`。
3. 桌面侧是在现有 caBLE 代码上的**增量**：Noise 握手、消息加解密
   （`handshake.rs` / `crypter.rs`）可原样复用，只需替换承载链路。`bluer`
   的 `l2cap` feature 直接提供 LE CoC 客户端，运行时不依赖 `bluetoothd`。
4. 最大的不确定性不在桌面 API，而在：advertisement suffix 能否经 BlueZ 拿到
   （iPhone 不发 suffix，此项仍未验证，需 Android）、CoC 模式（LEB vs ECFC）
   与手机的互通、以及消息分帧约定。

## 2. 协议机制（§11.5.1.1 与 §11.5.1.1.2）

1. **声明通道**：client（桌面）在 QR 的 `Key 6` 里列出支持的通道，`0`
   =Websockets、`1`=Bluetooth Low Energy；该键缺省视为 `[0]`。要启用 BLE
   必须在 QR 带上 `1`。
2. **authenticator 建通道**：手机取双方支持集的交集，若含 BLE，可创建一个
   **insecure L2CAP CoC server socket**，并自动分配一个 `server PSM`。
3. **经广播传 PSM**：手机把 PSM 写进 **advertisement suffix**——一个 CBOR map
   `{transport_channel_identifier: channel_extra}`，此处为 `{1: <PSM>}`——
   **追加在 20 字节 service data 之后**。规范明确：追加后缀需要 LE
   **extended advertising**。
4. **client 取 PSM**：client 仍只对前 20 字节 trial-decrypt 做 proximity 证明
   （与今天一致）；随后把 `serviceData[20:]` 当 CBOR 解析，取出 `Key 1` 的 PSM。
5. **连 PSM**：client 连该 PSM 的 LE CoC（不配对、insecure）。
6. **握手与消息层不变**：通道建立后是同一套 Noise `KNpsk0`（P-256 / SHA-256 /
   AES-256-GCM），PSK 由 QR secret + 解密后的 advert plaintext 派生。之后每条
   消息首字节为类型：`0` shutdown、`1` CTAP、`2` update、`3` JSON（与
   WebSocket 通道完全相同的消息层）。

## 3. 桌面侧实现

- **feature**：Cargo `l2cap`（启用 `bluer/l2cap`），默认关闭；运行时
  `UCABLED_BLE_CHANNEL=1` 才在 QR 里带上 BLE，默认仍走 WebSocket。
- **`advert.rs`**：解析 advertisement suffix 的 CBOR map，取
  `transport_channel_identifier = 1` 对应的 PSM（`parse_psm`）。
- **`ble.rs`**：新增 `await_advert_full`，回传
  `AdvertObservation { plaintext_eid, address, address_type, psm }`；trial-decrypt
  只喂前 20 字节（修掉了带 suffix 时按整段 20 字节校验会失败的问题）。
- **`qr.rs`**：新增 `Key 6` 编码（`TRANSPORT_WEBSOCKET` / `TRANSPORT_BLE`），
  解析器可跳过数组值。
- **链路抽象**：`phone::CableTransport`（WebSocket 或 L2CAP），`CableLink`
  改持 `transport`；`session::Channel` 决定走哪条。
- **`l2cap.rs`**：LE CoC 客户端 `connect` / `send` / `recv`，用
  `SOCK_SEQPACKET`（LE Credit Based，约定一 SDU = 一条 caBLE 消息）；连上后把
  安全等级设为 Sdp（insecure）、recv MTU 开到最大。
- **`relay.rs`**：advert 带 PSM 且已启用时选 L2CAP，否则 WebSocket。
- **`session.rs`**：L2CAP connect 带与 WebSocket dial 相同的 20s 超时；
  connect 失败或超时都回落 WebSocket 通道（规范允许手机同时等多条通道；
  若手机只听 BLE，回落由 dial 超时兜底）。握手阶段失败不回落。
- **复用**：`handshake.rs`（Noise）与 `crypter.rs`（消息加解密）未改。
- **测试**：suffix/PSM 与 QR `Key 6` 有单测；`tests/l2cap.rs` 有自环回 L2CAP
  测试与完整 mock-phone 往返（握手/消息层/CTAP 回合，需蓝牙适配器，默认
  `#[ignore]`）。

## 4. 未决问题（按风险排序）

| # | 问题 | 说明 |
| --- | --- | --- |
| 1 | **CoC 模式互通** | LE Credit Based Flow Control（包边界，Linux `SOCK_SEQPACKET`）还是 Enhanced Credit Based（字节流，`SOCK_STREAM`）。Fast Pair 明说用 "LE credit based flow control"，故实现选了 LEB；若手机实际用 ECFC，需把 `l2cap.rs` 换成 `Stream` 并定长度前缀。 |
| 2 | **extended advertising 后缀** | 后缀要求 LE 扩展广播。BlueZ 会解析扩展广播报告，理论上会进 `Device1.ServiceData`，但必须真机确认 `payload.len() > 20`。这是 go/no-go 点。 |
| 3 | **消息分帧** | 实现按「一 SDU = 一条 caBLE 消息」假设（规范示例用 `WriteMessage(BinaryMessage)`）；若对拷不符，需要自定义长度前缀。Chromium 尚未落地、Android 闭源，需真机对拷验证。 |
| 4 | **insecure CoC 免配对** | 规范明说 insecure，需确认内核允许不 bonding、安全等级可低，以及随机地址轮换下的连通性（Chromium 作者早提过未配对时 L2CAP 在 MAC 轮换下不稳定）。 |
| 5 | **手机/平台支持面** | **iPhone 实测不支持**（见 §5，含离线场景与 90s 重广播监听；iOS 27.0）。且 iOS 上第三方应用无法设置 BLE service data（fido-dev 邮件组），只有 Apple 能实现。Android 侧 Google 已官宣（安全工程博客 "Hybrid Transport Goes Offline!"，2026-03-04），GMS Core 闭源无法直接验证。**Chromium 桌面端未落地**：唯一 CL 7845133（仅 macOS、flag `FidoBleL2cap` 默认关）自 2026-05-13 卡在 tryjob 未合并；main 分支 `device/fido/cable/` 无任何 L2CAP/suffix/Key 6 代码。 |

## 5. 已做的验证与剩余

已做：iPhone 真机观测（`cargo run --bin cable-advert-probe`，QR 提供
`Key 6=[0,1]`）——

- 2026-10-02 与 2026-10-13（联网，iOS 27.0）：advert 均为 20 字节、无
  suffix/PSM，trial-decrypt 成功。
- 2026-10-13（iOS 27.0，飞行模式、仅蓝牙、WiFi 确认关闭）：**300 秒内完全没有
  advert**。原因：advert 的明文 EID 嵌着隧道服务器分配的 3 字节 routing
  ID，手机必须先连上隧道才能广播；离线时连隧道失败，手机转圈后报
  "操作无法完成"。支持 BLE 通道的手机此时应改用 dummy routing ID +
  PSM suffix 广播（routing ID 在 BLE 路径下用不到），故"离线无广播"
  本身就是无 BLE 通道的签名。
- 首次匹配后继续监听 90 秒（不限地址，覆盖 RPA 轮换）：无任何带 suffix
  的重广播。

剩余（需 Android 或未来 iOS）：

1. 对 Android 确认 advert 出现 `payload.len() > 20` 且 PSM 解析正确；若
   Android 也只见到 20 字节，需先用 btmon（需 root）排除桌面侧 BlueZ 丢
   extended-advertising 后缀的可能（§4 #2，iPhone 实验未能覆盖此点）。
2. 完成一次完整往返（可用 `tests/l2cap.rs` 的双设备 mock-phone 对先验证
   分帧/MTU 假设）；若握手后收不到消息，多半是 CoC 模式（LEB/ECFC）或
   分帧假设不符，按 §4 调整 `l2cap.rs`。

## 6. 参考

- FIDO CTAP 2.3 §11.5.1.1（data transfer channel）、§11.5.1.1.2（BLE）与
  §11.5.1.2（Data Transfer / Noise / 消息类型）
- Chromium issue `493286564`（BLE as a data transport channel for FIDO Hybrid）与
  CL `7845133`（桌面端唯一实现尝试，macOS-only，2026-05-13 起停滞）
- Google 安全工程博客 "Hybrid Transport Goes Offline!"（2026-03-04，Android
  侧官宣）
- fido-dev 邮件组：iOS 应用无法设置 BLE service data，hybrid 只能由平台实现
- FIDO `fido-2-specs` PR `#1668`（offline BLE channel）；2026-07 起 hybrid 章节
  拆分为独立的 PXP（Proximity Exchange Protocol）工作草案
- `bluer` `l2cap` 模块与示例 `l2cap_client.rs`
- `inabajunmr/hbtp-simple`（单文件 Go 版 hybrid，含 suffix PSM 解析，仍走 WS）
