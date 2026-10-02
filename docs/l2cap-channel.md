# BLE L2CAP 数据通道（CTAP 2.3 hybrid）

> 状态：桌面侧已实现（实验性，Cargo feature `l2cap`，运行时
> `UCABLED_BLE_CHANNEL=1`，默认仍走 WebSocket）。**2026-10-02 用 iPhone 实测：
> 即使 QR 提供 `Key 6=[0,1]`，iPhone 的 caBLE advert 仍只有 20 字节、无
> advertisement suffix/PSM**，即 iOS 不支持该 BLE/L2CAP 通道。故该特性目前只对
> （据 Chromium/Google 已实现的）Android 有意义；iOS 若日后支持则无需改协议。

## 1. 结论

1. CTAP 2.3 给 hybrid（§11.5）新增了 **BLE 数据通道**，走 insecure L2CAP
   CoC，把 CTAP 流量完全从隧道服务器上拿走。WebSocket 仍是默认与回退通道，
   两者二选一、不并发。
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
- **复用**：`handshake.rs`（Noise）与 `crypter.rs`（消息加解密）未改。
- **测试**：suffix/PSM 与 QR `Key 6` 有单测；`tests/l2cap.rs` 有自环回 L2CAP
  测试（需蓝牙适配器，默认 `#[ignore]`）。

## 4. 未决问题（按风险排序）

| # | 问题 | 说明 |
| --- | --- | --- |
| 1 | **CoC 模式互通** | LE Credit Based Flow Control（包边界，Linux `SOCK_SEQPACKET`）还是 Enhanced Credit Based（字节流，`SOCK_STREAM`）。Fast Pair 明说用 "LE credit based flow control"，故实现选了 LEB；若手机实际用 ECFC，需把 `l2cap.rs` 换成 `Stream` 并定长度前缀。 |
| 2 | **extended advertising 后缀** | 后缀要求 LE 扩展广播。BlueZ 会解析扩展广播报告，理论上会进 `Device1.ServiceData`，但必须真机确认 `payload.len() > 20`。这是 go/no-go 点。 |
| 3 | **消息分帧** | 实现按「一 SDU = 一条 caBLE 消息」假设（规范示例用 `WriteMessage(BinaryMessage)`）；若对拷不符，需要自定义长度前缀。Chromium 尚未落地、Android 闭源，需真机对拷验证。 |
| 4 | **insecure CoC 免配对** | 规范明说 insecure，需确认内核允许不 bonding、安全等级可低，以及随机地址轮换下的连通性（Chromium 作者早提过未配对时 L2CAP 在 MAC 轮换下不稳定）。 |
| 5 | **手机支持面** | **iPhone 实测（2026-10-02）不支持**：QR 带 `Key 6=[0,1]` 时其 advert 仍是 20 字节、无 suffix/PSM（用 `cargo run --bin cable-advert-probe` 观测，trial-decrypt 成功）。Android 据 Chromium issue `493286564` 已实现、待实测；Chromium 桌面端仍在开发中。 |

## 5. 已做的验证与剩余

已做：iPhone（2026-10-02）用 `cargo run --bin cable-advert-probe` 观测——QR 提供
`Key 6=[0,1]` 时 advert 仍为 20 字节、无 PSM，trial-decrypt 成功，即 iOS 不支持。

剩余（需 Android 或未来 iOS）：

1. 对 Android 确认 advert 出现 `payload.len() > 20` 且 PSM 解析正确。
2. 完成一次完整往返；若握手后收不到消息，多半是 CoC 模式（LEB/ECFC）或分帧
   假设不符，按 §4 调整 `l2cap.rs`。

## 6. 参考

- FIDO CTAP 2.3 §11.5.1.1（data transfer channel）、§11.5.1.1.2（BLE）与
  §11.5.1.2（Data Transfer / Noise / 消息类型）
- Chromium issue `493286564`（BLE as a data transport channel for FIDO Hybrid；
  含 L2CAP CoC 与 `advertisement suffix` 实现要点）
- FIDO `fido-2-specs` PR `#1668`（offline BLE channel）
- `bluer` `l2cap` 模块与示例 `l2cap_client.rs`
- `inabajunmr/hbtp-simple`（单文件 Go 版 hybrid，含 suffix PSM 解析，仍走 WS）
