# BLE L2CAP 数据通道（CTAP 2.3 hybrid）：可行性调查

> 状态：调研完成，尚未实现。桌面侧技术上可行（BlueZ/bluer 支持 LE L2CAP
> CoC），但有若干待真机验证的互通点。手机侧 Android 已实现；iOS 未确认。
> 它是 WebSocket 隧道服务器的**可选替代**，面向离线与隐私场景。

## 1. 结论

1. CTAP 2.3 给 hybrid（§11.5）新增了 **BLE 数据通道**，走 insecure L2CAP
   CoC，把 CTAP 流量完全从隧道服务器上拿走。WebSocket 仍是默认与回退通道，
   两者二选一、不并发。
2. 它与 §11.4 的 FIDO GATT service（`0xFFFD`）**无关**；GATT 与本项目手机的
   关系见 `docs/gatt-data-channel.md`。
3. 桌面侧是在现有 caBLE 代码上的**增量**：Noise 握手、消息加解密
   （`handshake.rs` / `crypter.rs`）可原样复用，只需替换承载链路。`bluer`
   的 `l2cap` feature 直接提供 LE CoC 客户端，运行时不依赖 `bluetoothd`。
4. 最大的不确定性不在桌面 API，而在：advertisement suffix 能否经 BlueZ
   拿到、CoC 模式（LEB vs ECFC）与手机的互通、以及消息分帧约定。

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

## 3. 桌面侧改动面

- **bluer**：当前只开 `bluetoothd` feature；需增加 `l2cap`。`bluer::l2cap`
  提供 `Stream` / `SeqPacket` / `StreamListener` 与
  `SocketAddr { addr, addr_type, psm }`，官方示例
  `Stream::connect(SocketAddr::new(addr, AddressType::LePublic, psm))` 即 LE
  客户端连 PSM；走内核 socket，运行时不依赖 `bluetoothd`。
- **`ble.rs`**：`await_advert` 现在只回传 16 字节明文，丢弃了设备地址、地址
  类型与后续 suffix。需要改为回传 `(address, addr_type, plaintext, suffix)`，
  并解析 `serviceData[20:]` 的 `{1: psm}`。`device.service_data()` 已取得整段
  payload，改动局限在返回类型与解析。
- **链路抽象**：`phone.rs::CableLink` 直接持有 `ws` 并调用
  `隧道::write_binary/read_binary`；`session.rs::DesktopFlow` 握手前后也直接用
  `ws`。需引入一个字节/消息双向流 trait，让 WebSocket 与 L2CAP 两种实现可换。
- **复用**：`handshake.rs`（Noise）与 `crypter.rs`（消息加解密）无需改动。

## 4. 未决问题（按风险排序）

| # | 问题 | 说明 |
| --- | --- | --- |
| 1 | **CoC 模式互通** | LE Credit Based Flow Control（包边界，Linux `SOCK_SEQPACKET`）还是 Enhanced Credit Based（字节流，`SOCK_STREAM`）。Fast Pair 明说用 "LE credit based flow control"；bluer 两种都提供，需实测与 Android/iOS 哪种互操作。 |
| 2 | **extended advertising 后缀** | 后缀要求 LE 扩展广播。BlueZ 会解析扩展广播报告，理论上会进 `Device1.ServiceData`，但必须真机确认 `payload.len() > 20`。这是 go/no-go 点。 |
| 3 | **消息分帧** | 规范示例把通道当消息导向用（`WriteMessage(BinaryMessage)`）。若一 SDU = 一条 caBLE 消息则无需前缀，否则要自定义长度前缀。Chromium 尚未落地、Android 闭源，需对拷验证。 |
| 4 | **insecure CoC 免配对** | 规范明说 insecure，需确认内核允许不 bonding、安全等级可低，以及随机地址轮换下的连通性（Chromium 作者早提过未配对时 L2CAP 在 MAC 轮换下不稳定）。 |
| 5 | **手机支持面** | Android 已实现（见 Chromium issue `493286564`，动机含离线与 EU DC API 隐私要求）；iOS 有 `CBL2CAPChannel` 但 iCloud Keychain authenticator 是否实现未确认；Chromium 桌面端仍在开发中。 |

## 5. 建议的 spike（可先在本地做，不需要手机）

1. 扩 `ble.rs`：回传 `(address, addr_type, plaintext, suffix)`，解析 PSM 仅打
   日志；先验证真机是否出现 `payload.len() > 20`。
2. 本地 mock：用 `bluer` 起一个带 suffix 的 advertiser + `l2cap`
   `StreamListener`（LE CoC），跑通「连 PSM → Noise → getInfo」，验证 Linux
   客户端路径与分帧假设。
3. 真机：QR 的 `Key 6` 置 `[0,1]`，对已支持 BLE 通道的 Android 验证后缀与 PSM
   是否出现，并完成一次完整往返。

## 6. 参考

- FIDO CTAP 2.3 §11.5.1.1（data transfer channel）、§11.5.1.1.2（BLE）与
  §11.5.1.2（Data Transfer / Noise / 消息类型）
- Chromium issue `493286564`（BLE as a data transport channel for FIDO Hybrid；
  含 L2CAP CoC 与 `advertisement suffix` 实现要点）
- FIDO `fido-2-specs` PR `#1668`（offline BLE channel）
- `bluer` `l2cap` 模块与示例 `l2cap_client.rs`
- `inabajunmr/hbtp-simple`（单文件 Go 版 hybrid，含 suffix PSM 解析，仍走 WS）
