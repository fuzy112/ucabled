# BLE 数据通道：GATT 与 L2CAP

> 状态：在 CTAP 2.3（2026-02 发布）下复核完成。结论有两层：
> 1. FIDO GATT service（UUID `0xFFFD`，CTAP 2.0 编号 §8.3.5，现 §11.4.5）是
>    规范定义的、能承载 CTAP2 的独立 BLE 传输绑定；但 iOS / Android 平台
>    passkey **不暴露**该 GATT server。
> 2. CTAP 2.3 为 hybrid 新增了 BLE 数据通道，走 **L2CAP CoC**（不是 GATT）。
>    FR-8b 原设想的"GATT 数据通道"不可行；真正可作为隧道服务器替代品的是
>    L2CAP 通道，它的桌面侧可行性待评估。

## 1. 结论

1. **GATT service 确实存在于规范中**（§11.4 "BLE" 绑定）：authenticator 是
   GATT server、client 是 GATT client，帧为 PING / KEEPALIVE / MSG / CANCEL，
   MSG 的 payload 即 §8 Message Encoding——也就是 **CTAP2 CBOR**（不只是
   U2F）。用不用它是平台实现选择，不是协议缺陷。
2. iOS（iCloud Keychain）与 Android（Google Password Manager）作为平台
   authenticator **不暴露**这个 GATT server，只实现 hybrid。
3. hybrid（§11.5）的数据通道有 WebSocket（默认、回退）与 BLE（2.3 新增）两种；
   **BLE 通道用 insecure L2CAP CoC，不经过任何 GATT service**。即"hybrid 无
   GATT 数据通道"仍然成立，但"hybrid 无 BLE 数据通道"从 2.3 起不再成立。
4. 因此，面向"手机 passkey、零安装"的场景：GATT 通道不可用（手机不提供）；
   若要摆脱隧道服务器，应评估 L2CAP 通道，而非 GATT。

## 2. 依据

### 2.1 FIDO GATT service（§11.4.5；CTAP 2.0 为 §8.3.5）

- 角色：FIDO Client = GATT client，FIDO Authenticator = GATT server。
- 服务 UUID `0xFFFD`（Primary Service）。特征：

  | 特征 | 属性 | UUID |
  | --- | --- | --- |
  | `fidoControlPoint` | Write | `F1D0FFF1-DEAA-ECEE-B42F-C9BA7ED623BB` |
  | `fidoStatus` | Notify | `F1D0FFF2-DEAA-ECEE-B42F-C9BA7ED623BB` |
  | `fidoControlPointLength` | Read (2B) | `F1D0FFF3-DEAA-ECEE-B42F-C9BA7ED623BB` |
  | `fidoServiceRevisionBitfield` | Read/Write | `F1D0FFF4-DEAA-ECEE-B42F-C9BA7ED623BB` |
  | `fidoServiceRevision` | Read | `0x2A28` |

- 帧格式 `CMD / HLEN / LLEN / DATA`；命令 `PING 0x81`、`KEEPALIVE 0x82`、
  `MSG 0x83`、`CANCEL 0xBE`、`ERROR 0xBF`。分片分 initialization fragment
  （首字节高位置 1，后跟两字节大端总长）与 continuation fragment（SEQ，
  `0x00..0x7F` 回绕）。
- `fidoServiceRevisionBitfield` 的 bit 5 即 **FIDO2**（bit 6/7 为 U2F 1.2/1.1），
  且 §11.4.4.1 明确写着 MSG 的 data format 是 §8 Message Encoding。**所以该
  GATT 绑定可承载 CTAP2，先前"只有 U2F/CTAP1、无 CBOR"的说法有误。**
- 手机不提供该 server，故无法与 iOS / Android 平台 passkey 互通。

### 2.2 hybrid 的数据通道（§11.5.1.1）

- §11.5.1.1.1 WebSocket（CTAP 2.2 起）：隧道服务器中转，URL 为
  `wss://<domain>/cable/connect/<routingID>/<tunnelID>`，子协议 `fido.cable`。
  仍是默认与回退通道。
- §11.5.1.1.2 BLE（CTAP 2.3 新增）：authenticator 建 insecure L2CAP CoC
  server socket 并自动生成 `server PSM`；client 在 QR 的 `Key 6`（支持的通道
  列表，`0`=Websockets、`1`=BLE）里声明支持后，authenticator 把 PSM 放进
  **advertisement suffix**——即追加在 20 字节 service data 之后的 CBOR map
  `{transport_channel_identifier: channel_extra}`，此处为 `1: <PSM>`——client
  据此连 L2CAP。两个通道二选一，不并发；连上后丢弃其它尝试。
- 两种通道之上都是同一套 Noise `KNpsk0`（P-256 / SHA-256 / AES-256-GCM）与
  CTAP2 消息流，区别只在承载字节的链路。

### 2.3 Chromium

- `device/fido/cable/` 在 HEAD 已只剩 v2（`websocket_adapter`、`v2_*`、
  `fido_tunnel_device`），没有任何 GATT 数据链路；正在实现 hybrid 的 BLE 数据
  通道（Chromium issue `493286564`，采用 L2CAP CoC，而非 GATT）。
- 独立的 BLE 安全密钥传输 `device/fido/ble/` 已删除，caBLE v1 的 GATT 连接
  （旧 `cable/fido_ble_connection.cc`）也不在了；`fido_ble_uuids` 里的 `0xFFFD`
  只是旧 "BLE" GATT 传输的遗留。README 仍（过时地）写着支持 BLE 安全密钥：
  历史上 Chromium 桌面版曾以 `WebAuthenticationBle`
  （`chrome://flags#enable-web-authentication-ble-support`）支持，现已被移除。

### 2.4 本项目现状

- `CABLE_BLE_UUID`（`0000fff9`）在 caBLE v2 中只用于广播 service data
  （`ble.rs::scan`），不是数据通道 UUID。
- 我们只实现广播接收（FR-8a），既没有 GATT，也没有 L2CAP。

### 2.5 平台侧证据：手机不作为 GATT authenticator

没有平台文档或实现把 §11.4 的 GATT service 当作 passkey 通道；手机作为
authenticator 一律走 hybrid。直接证据：

- **Chromium caBLE 作者 Adam Langley（2021）**：Google 账号的"手机当安全密钥"
  曾 "functioned over BLE GATT between the desktop and phone"，但 "the success
  rate that we measured with BLE was poor"，最终改为 "all the communication
  happens over the internet connection, but the phone sends a nonce in a BLE
  advert"，并提到未来可能加 L2CAP。这是"GATT 被主动放弃"的第一手记录。
  <https://www.imperialviolet.org/2021/10/20/cablev2.html>
- **Chromium issue 493286564**：现状是 "Chromium only supports the tunnel server
  (WebSockets) as the data transport channel, utilizing BLE solely for proximity
  proof and handshaking"；其在做的 BLE 数据通道是 L2CAP CoC（与 Android 对齐）。
  <https://issues.chromium.org/issues/493286564>
- **Chromium issue 372553135 "Remove caBLEv1 and server-linked v2 support"**
  （2026-02）：caBLE v1 那条 GATT 路径（且是 Chromium 私有协议，不是 0xFFFD）
  正在移除。
  <https://issues.chromium.org/issues/372553135>
- **webauthn-authenticator-rs 的 cable 文档**：initiator 与 Android / iOS 16
  authenticator 的协议是 BTLE advert + WebSocket 隧道；Android 上由 Chrome 处理
  `FIDO:/`、建隧道并转发到 Google Play FIDO2 API，iOS 上是 iCloud Keychain，
  全程无 GATT。
  <https://docs.rs/webauthn-authenticator-rs/latest/webauthn_authenticator_rs/cable/index.html>
- **Apple**：官方 Security Keys 支持文档只列 NFC / USB-C / Lightning / USB-A
  连接器，不含蓝牙；`AuthenticationServices` 里虽有 `Transport.bluetooth` 这个
  WebAuthn 枚举值，但不代表 iOS 提供 GATT authenticator。跨设备时 Apple 用的是
  hybrid（把已登录的受信设备靠近）。
  <https://support.apple.com/en-us/102637>
- **Android（client 侧）**：`Transport` 枚举仍保留 `BLUETOOTH_CLASSIC` /
  `BLUETOOTH_LOW_ENERGY`，历史上支持过蓝牙硬件密钥（如 Google Titan 蓝牙版），
  但这是 Android 作为 GATT client 连硬件密钥；作为 authenticator，Android 平台
  不暴露 0xFFFD，只有第三方 app 才提供。
  <https://developers.google.com/android/reference/com/google/android/gms/fido/common/Transport>
- **第三方实验**：Android 上要暴露 0xFFFD GATT server 必须自己写 app，例如
  `kshoji/Android-BLE-FIDO2-Authenticator`（"Experimental ... not a certified
  authenticator"）——反证平台不提供。

诚实的边界：没有厂商明文写"我们不实现 0xFFFD"。结论依据是：官方"受支持传输"
清单（NFC / USB / hybrid）不含它 + 无公开的平台 API + 历史尝试被放弃 + 需要
第三方 app 补位。另注意 Android 现已在 hybrid 内实现 L2CAP BLE 数据通道，故
"手机只用 WebSocket"已不再严格成立，但那仍是 L2CAP，不是 GATT。

## 3. 若要在桌面侧启用 L2CAP 通道

见专门的 `docs/l2cap-channel.md`（协议机制、`bluer` 的 L2CAP 支持、未决问题与
spike 计划）。要点：把承载链路换成 LE L2CAP CoC，其余 caBLE 代码可复用。

## 4. 建议

- 重写 FR-8b：不是"BLE GATT 数据通道不存在"，而是"平台 passkey 不暴露 §11.4
  的 GATT server；hybrid 的可选 BLE 通道是 L2CAP，作为摆脱隧道服务器的候选
  待评估"。
- 隧道服务器仍是当前唯一已实现、且所有目标手机都支持的数据通道。
