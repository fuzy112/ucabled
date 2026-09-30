# 同类项目调研与可借鉴点

> 目的：记录与本项目同域的开源实现，以及其中值得借鉴的具体做法。
> 条目是"候选改进",采纳与否需单独评估；**未经实测的不作为既定结论**。
>
> 审阅日期：2026-09-30；下列每个项目的审阅版本（分支 HEAD）在其小节标注。

## 1. passkeyd（bjn7/passkeyd）

审阅版本：`477b5e2`（2026-09-21）。

**本机软件 WebAuthn 认证器**（softoken）：passkey 存储在本地，支持 TPM 与
非 TPM，带 PAM 集成、`passkeyd-manager` CLI、Ice/KDE/GTK 主题。

与本项目是**相反的信任模型**（密钥留本机），对应 plan.md §1 中我们刻意不
走的路线（KeePassXC / soft-fido2 一类）。仅作"本机认证器 / UI / PAM 生态"
参考，不涉及手机中继。

## 2. cable-uhid-bridge（baseman70/cable-uhid-bridge）

审阅版本：`51096d8`（2026-09-28）。

**与 ucabled 目标、架构几乎相同的平行实现**：`/dev/uhid` 虚拟 FIDO2 +
caBLE v2 QR + 手机 passkey + egui 子进程浮窗。其真机验证覆盖 iPhone 与
Android（含 cancel 流程），RP 覆盖 GitHub / webauthn.io / Google Accounts。

### 2.1 值得评估的改进（按性价比排序）

1. **getInfo 广告 0x07 / 0x08**（`maxCredentialCountInList`、
   `maxCredentialIdLength`）。其实现注释（`src/ctap2.rs` 的
   `build_get_info_response`）指出：不广告这两个值时，Chromium/Firefox 会把
   allowList 拆成 size-1 批次并发送 up=false 静默探测，可能引起认证失败与
   dummy touch 回退。本项目 getInfo 目前只有键 1/3/4/5/9。
2. **转发前剥掉凭据描述符的 `transports`**（allowList 与 excludeList 都处理，
   只保留 `id`/`type`）。其 `sanitize_credential_descriptor` 注释说明：
   Firefox 会给凭据打 `transports: ["usb"]`,原样转发到 caBLE 会被 iOS 以
   transport 不匹配为由拒绝，报 "No passkeys found"。本项目 plan.md §5.3 已
   记录 `transports` 恒为 `usb` 的怪癖，但把它当作"纯标签问题"直接透传。
3. **Assertion 缓存 / 两段式重放**（`src/engine.rs` 的 `CachedAssertion`,
   TTL 10s）：成功后按 `rp_id` + `clientDataHash`(+credential id) 缓存响应，
   浏览器紧接着发来的第二次定向 getAssertion 直接重放，避免再次弹二维码。
4. **取消冷却**（`src/engine.rs`,`cooldown_duration` 默认 5s）：取消后对后续
   请求直接回 `CTAP2_ERR_KEEPALIVE_CANCEL`,避免浏览器立刻重试。
5. **dummy 探测集合**：它拦截 `make.me.blink` / `.dummy` / `dummy`。本项目只
   识别 `make.me.blink`,且走"Use phone"选择窗口——与实体密钥共存这一点上
   本项目更完整，不建议退回直接拒绝。

### 2.2 观察项（不推荐照抄）

- **丢弃全部 extensions**（makeCredential key 6 / getAssertion key 4），并强制
  `options={rk:true,uv:true}` / `{up:true,uv:…}`、丢弃 pinAuth；理由是其认为
  手机 GetInfo 无 extensions、严格解析器会失败。本项目是透传 + 仅做 iOS 必需
  的最小注入，worklog 记录的 A/B/C 实验也未显示去 extensions 有效。存疑。
- **getInfo 直接使用 Apple 真机 AAGUID**(`f24a8e70-d0d3-f82c-2937-32523cc4de5a`)。
  本项目使用自己的固定 AAGUID,更干净。

### 2.3 架构与安全对比

| 维度 | ucabled | cable-uhid-bridge |
| --- | --- | --- |
| 权限模型 | **system service**,专用 `ucabled` 用户独占 `/dev/uhid`,polkit 限活动本地会话 | systemd **user service** + `/dev/uhid` `uaccess`（任意用户进程可创建虚拟 HID） |
| caBLE 实现 | 手写，逐函数对齐 Chromium | 内嵌 `webauthn-authenticator-rs` 的 `cable`（含 `cable-override-tunnel`） |
| 真机验证 | iOS ✅ / Android ❌ | iOS ✅ / Android ✅ |
| 打包 | Nix flake + module | installer + deb / AUR / tarball |
| 许可 | GPL-3.0-or-later | MIT OR Apache-2.0 |

其权限模型是本项目早期就收敛掉的（见 docs/system-service.md §1），不拟跟进；
其打包/多发行版安装脚本、Snap/AppArmor 排障说明可作参考。

## 3. 待跟进

- 对方**验证了 Android**:可去读 authenticator-rs 的 `cable` 实现，对照本项目
  手写实现，找出 Android 在 transports / extensions / options 上的差异，为
  Android 真机复测做准备。
- authenticator-rs 的 `cable-override-tunnel` 是 **initiator 侧**覆盖隧道 URL，
  与 docs/tunnel-server.md 的结论不冲突（手机仍自选域名）。
- 以上 2.1 的 1、2 改动小、针对真实登录失败，风险低；3、4 属体验改进，可另评。
