# BLE GATT 数据通道：可行性调查

> 状态：调查完成。**结论：caBLE v2 / hybrid 协议里不存在 BLE GATT 数据通道，
> FR-8b 建议关闭。** 原设想（桌面作 GATT central 连手机的 GATT server，作为
> 隧道服务器的替代传输）在本项目依赖的手机 passkey 协议中没有对应定义。

## 1. 结论

1. hybrid（CTAP 2.2 §11.5）的传输只有两件事：**WSS 隧道服务器**承载全部
   CTAP2 加密流量，**BLE 广播**只用于邻近性证明（EID）。没有 GATT 数据通道。
2. 规范里唯一的 BLE GATT FIDO 传输是 **§11.4（Bluetooth Smart / BLE）**，即旧
   的 U2F/CTAP1-over-BLE：service UUID `0xFFFD`，**authenticator 是 GATT
   server**、client 是 GATT client，帧格式只有 PING / MSG / CANCEL（§8 的
   APDU 编码），**没有 CTAP2 / CBOR**。
3. iPhone（iCloud Keychain）与 Android（Google Password Manager）**不暴露
   这个 GATT server**，也不使用它。它们只走 hybrid。

因此"实现 BLE GATT 数据通道"要么是在实现一个手机不会响应的自定义协议（双方
都不可控，无法互通），要么是在实现与 passkey 无关的旧 U2F BLE 传输。

## 2. 依据

### 2.1 规范（CTAP 2.2 §11.5）

§11.5 开头明确："It involves both network communication via a service called
a tunnel service, and BLE transmissions to show proximity." 之后 §11.5.1
QR 流程的全部内容都是：QR（含 identity 公钥 + secret）→ BLE 广播（EID,含
nonce/routing ID/tunnel service identifier）→ `wss://<domain>/cable/connect/
<routingID>/<tunnelID>`，子协议 `fido.cable`。文档在建立隧道后直接进入
Noise 握手与 CTAP2，全程没有 GATT 数据通道。

EID 里的 16 位 "tunnel service identifier" 只用于**选择隧道服务器域名**
（assigned 表下标或 ≥256 的哈希域名），不是"选择传输方式"。

传输枚举（§11.1）也把两者分开：

- `BLE`, when using the FIDO GATT service.
- `HYBRID`, when using the FIDO Hybrid service.

即 hybrid 不含 BLE 数据通道；`BLE` 指的就是 §11.4 的 U2F GATT 传输。

规范还说明隧道服务器与 authenticator 之间的协议是"a private detail of the
authenticator's implementation"——它本就不开放给第三方桌面端。

### 2.2 Chromium 实现

- `device/fido/cable/` 里唯一的数据链路是 `websocket_adapter.{h,cc}`（隧道）；
  没有任何 GATT 数据通路。
- `device/fido/ble/` 目录已不存在；`device/fido/` 下与 BLE 相关的只剩
  `ble_adapter_manager`（管理广播/发现）。
- `fido_ble_uuids.{h,cc}` 里的 `0xFFFD` FIDO service 属于旧的 U2F BLE 传输。

### 2.3 本项目现状

`0000fff9`（`lib.rs:CABLE_BLE_UUID`）在 caBLE v2 中**只**用于广播的 service
data（`ble.rs::scan` 我们已实现）；它不是数据通道的服务 UUID。

## 3. 假如协议存在，改动面会有多大（供参考）

即便抛开"手机不响应"这一点，桌面侧要做到"用 BLE 承载加密 CTAP 帧"也需要：

- 抽出链路抽象：`phone.rs::CableLink` 目前直接持有 `ws: Ws` 并调用
  `tunnel::write_binary/read_binary`；`session.rs::DesktopFlow` 在握手前后都
  直接用 `ws`。需要引入一个字节双向流 trait 才能替换链路。
- 新写一个 GATT central（bluer）：连接、服务/特征发现、写 Control Point、
  订阅 Status/通知、按 MTU 分片与重组。
- Noise 握手、消息加解密（`handshake.rs`/`crypter.rs`）可原样复用。

但手机侧不可控，做出来也无法互通，所以没有意义。

## 4. 建议

- **关闭 FR-8b**：它不是"预期不需要"，而是"协议中不存在 / 手机不提供"。
- 若真正的目标是摆脱隧道服务器或减少元数据暴露，另有独立的结论：见
  `docs/tunnel-server.md`（同样不可行）。
