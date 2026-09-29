# 隧道服务器：为何不自建（设计决定）

> 状态：结论已定，**不作为目标**。前提是 stock iOS / Android 客户端；
> 若将来使用可控客户端（可编译的 Chromium、自研 authenticator 等），可
> 以重新评估，见文末。

## 1. 结论

caBLE v2 的隧道服务器由**手机端选择并硬编码**，二维码里没有任何字段可以
指定一个自建服务器；桌面端只能连接手机选定的那个域名。因此本项目不自建
隧道服务器，也不把隧道当作可信组件（本来也不必）。

## 2. 协议依据

**二维码（桌面 → 手机）只携带"已知域名个数"，不携带域名。** Chromium 生成
二维码时直接写入固定常量表的长度：

```cpp
// device/fido/cable/v2_handshake.cc
qr_contents.emplace(2, static_cast<int64_t>(std::size(tunnelserver::kAssignedDomains)));
```

`kAssignedDomains = {"cable.ua5v.com", "cable.auth.com"}` 是编译进双方客户端
的常量表，映射到下标 0..255;≥256 才是哈希派生的自定义域名
`cable.<base32>.<tld>`（本项目对应 `src/tunnel.rs` 的
`decode_tunnel_server_domain`，并已支持 ≥256 的哈希形式）。

**选哪个域名的是手机。** 手机在 BLE advert 的 EID 里填入
`tunnel_server_domain`（`src/eid.rs`），桌面解密后照此连接
（`src/session.rs`）。Chromium 手机端的域名是写死的，注释说得很直白：

> `kTunnelServer` is the hardcoded tunnel server that phones will use for
> network communication. This specifies a Google service and the short domain
> seed is necessary to fit within a BLE advert.
> —— `device/fido/cable/v2_constants.h`

域名种子要能塞进 20 字节的 BLE advert，这个设计约束正是"域名不可协商"的
原因。ucabled 的二维码广告 `num_known_domains = 2`（`src/qr.rs`、
`src/relay.rs`），与 Chromium 一致，都只是告知手机"桌面认识哪些固定域名"。

## 3. 安全模型：自建的收益有限

隧道流量是**端到端加密**的：Noise KNpsk0（P-256 ECDH + PSK，PSK =
HKDF(QR secret, salt=解密后的 EID)）之上再走 AES-256-GCM。PSK 同时绑定
QR secret 与 BLE advert，因此隧道服务器既无法解密，也无法在不知道二者的
情况下插入或伪造 CTAP 消息。

服务器**能**看到的只有连接元数据（双方 IP、时间、消息长度、随机的
routing/tunnel ID),以及可用性——它可以拒绝服务或丢包。它看不到明文、
passkey 或签名，也无法伪造响应（响应由手机签名）。

代码本身也把它当作不可信输入：单帧上限 64 KiB（`src/tunnel.rs` 注释明确
"隧道服务器不被信任，不能让驱动它任意分配内存"）、TLS 1.3-only、dial / 读
/ 握手超时。换言之，自建服务器换来的是元数据不外泄与不依赖第三方可用性，
而不是更高的凭证安全性。

## 4. 唯一可行的自建路径及其代价

在不更换手机客户端的前提下，只有"劫持手机已经会拨的域名"这一条路：

1. **DNS**：用 Tailscale 的 split DNS（或 iPhone 上的 DNS 配置档）把
   `cable.auth.com` / `cable.ua5v.com` 解析到桌面所在的 tailnet 地址。这一步
   靠 Tailscale 是可行的。
2. **TLS**：绕不过去。公共 CA 不会给这两个域名签发证书，只能在 **iPhone
   上安装并完全信任一个自定义根 CA**，再由它签发该域名的证书——本质上让
   手机接受一次可信任的中间人。
3. **服务端**：daemon 在该域名上实现隧道中继（`/cable/new/<tunnelID>`、
   `/cable/connect/<routingID>/<tunnelID>`,按随机 ID 配对两条 WebSocket 并
   转发帧）。服务器是"哑中继",不接触 Noise 明文，实现不难。

代价很明确：往手机里注入根 CA 会**降低整机安全**（该 CA 签发的任何证书都
被信任），配置脆弱，Apple 的解析/校验行为随时可能改变，而且如 §3 所述
并不会提升凭证的机密性或完整性。**因此不推荐，也不实现。**

## 5. 何时重新评估

只有在能控制手机端客户端时才值得做：

- 可编译的 Chromium / 自研 authenticator：走 ≥256 的自定义域名
  （`cable.<base32>.<tld>`),自建服务器 + 真实证书即可。桌面端
  `decode_tunnel_server_domain` 已经支持自定义域名，无需改动。
- 某个平台若开放"自定义隧道服务器"的官方配置，再按该配置接入。

在此之前，使用 Google / Apple 的 assigned 隧道是唯一可行且可接受的方案。
