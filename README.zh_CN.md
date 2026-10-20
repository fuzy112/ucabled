# ucabled — 手机通行密钥桥

[![English](.github/badges/readme-en.svg)](README.md) · [![简体中文](.github/badges/readme-zh_CN.svg)](README.zh_CN.md)&emsp;&emsp;[![Rust](https://github.com/fuzy112/ucabled/actions/workflows/rust.yml/badge.svg)](https://github.com/fuzy112/ucabled/actions/workflows/rust.yml) [![Audit](https://github.com/fuzy112/ucabled/actions/workflows/audit.yml/badge.svg)](https://github.com/fuzy112/ucabled/actions/workflows/audit.yml)

> **注意：** 本项目完全由大语言模型（LLM）生成。

ucabled 让你把手机（iPhone / Android）当作 Linux 桌面上的 FIDO2 安全密钥。
当网站或 SSH 客户端请求通行密钥时，屏幕会弹出一个二维码；用手机扫码并用
Face ID 或指纹确认即可。无需改动浏览器，无需扩展，机器上也不存任何密钥
材料——通行密钥始终只存在于手机上。

底层上，它向内核注册一个虚拟的 USB HID FIDO2 设备（`/dev/uhid`），并通过
**caBLE v2**（二维码 + BLE 距离证明 + WSS 隧道，即 CTAP 2.2 §11.5 的 hybrid
传输）把浏览器发来的 WebAuthn 注册/登录请求中继到手机，再把手机的响应送回
浏览器。

```
      ┌─────────────┐
      │   Browser   │
      │  WebAuthn   │
      └──────┬──────┘
             │  CTAP-HID over /dev/uhid
             V
  ┌────────────────────┐      caBLE v2: BLE + WSS      ┌──────────────────┐
  │      ucabled       │<─────────────────────────────>│ iPhone / Android │
  │  (system service)  │                               │  passkey store   │
  └──────────┬─────────┘                               └────────┬─────────┘
             │  D-Bus org.ucabled.Manager1                      ^
             │  Prompt / Found / Close                          .
             V                                                  .
  ┌────────────────────┐                                        .
  │    ucable-agent    │                   phone scans          .
  │  (session agent)   │                   the QR code          .
  └──────────┬─────────┘                                        .
             │  spawn (QR URL via stdin)                        .
             V                                                  .
  ┌─────────────────────┐                                       .
  │ ucable-agent-helper │ . . . . . . . . . . . . . . . . . . . .
  │     QR window       │
  └─────────────────────┘
```

你的电脑只是中继：不在磁盘上保存任何密钥，也不做 FIDO 密码学运算。唯一的例外是少量
本地兼容性应答与改写——getInfo、Firefox 的探测请求，以及 iOS 必需的
`rp.name`/`user.displayName` 注入。解析 rpId 是只读的，仅用于给二维码窗口
取标题。

## 状态

- 已在 NixOS + Firefox + iPhone 上验证注册与登录，覆盖三款测试过的 iOS
  通行密钥提供方：iCloud 钥匙串、Google 密码管理器和 Strongbox。
- Android（Google 密码管理器）使用同一协议，预计可用，但未在真机上测试。
- 需要 Linux（`/dev/uhid`）、蓝牙，以及能看到 hidraw 的浏览器。Flatpak/Snap
  浏览器需要额外的 udev 配置；原生 NixOS 包开箱即用。

## 安装（NixOS，推荐）

把这个仓库加进你的 flake 并启用模块：

```nix
{
  inputs.ucabled.url = "github:fuzy112/ucabled/master";

  # 在你的 NixOS 模块中：
  imports = [ inputs.ucabled.nixosModules.ucabled ];
  services.ucabled.enable = true;

  # 可选：使用其他 UI helper
  services.ucabled.helper = "${pkgs.ucable-agent-helper-gnome}/bin/ucable-agent-helper";

  # 可选：额外的 helper 参数。内置 GTK4 helper 支持
  # --layer-shell，在 Sway、Hyprland 等 wlroots 合成器上
  # 把提示显示为 wlr-layer-shell 覆盖层：
  # services.ucabled.helperExtraArgs = [ "--layer-shell" ];
}
```

模块会配置好一切：

- 专用的 `ucabled` 用户和用户组
- 一条 udev 规则，通过 ACL 给 `ucabled` 账户对 `/dev/uhid` 的读写权限
  （`RUN+="... setfacl -m u:ucabled:rw /dev/uhid"`），不改动该设备的属组
- `boot.kernelModules = [ "uhid" ]`
- `hardware.bluetooth.enable = true`（`mkDefault`，可覆盖；caBLE 需要 BLE
  广播）
- `systemd.services.ucabled` 服务
- 为每个用户启用的 `systemd.user.services.ucable-agent` 会话代理
- D-Bus 策略和 polkit 动作 `org.ucabled.register-agent`（BlueZ 不需要
  polkit 规则）

执行 `nixos-rebuild switch`，然后注销再登录（或运行
`systemctl --user start ucable-agent`），让代理完成注册。

### 二进制缓存

`master` 发布到 `ucabled.cachix.org`，因此 Nix 会直接替换该包而无需构建。
在 NixOS 上：

```nix
nix.settings = {
  extra-substituters = [ "https://ucabled.cachix.org" ];
  extra-trusted-public-keys = [
    "ucabled.cachix.org-1:iEDAC8e63hzBZ/HKtMibPzl6kvTnPblcFs4+C50XvwU="
  ];
};
```

在其他系统上，`cachix use ucabled` 会把同样两项设置写入
`$HOME/.config/nix/nix.conf`。

每次向 `master` 推送时 CI 都会填充缓存。只缓存本仓库的构建产物（依赖仍来自
`cache.nixos.org`），且替换要求相同的提交与 `flake.lock`，因此本地改过的
源码树照常本地构建。

## 安装（Debian / Ubuntu）

源码树自带 Debian 打包文件，用标准工具即可构建 `.deb`：

```bash
sudo apt install debhelper cargo rustc pkgconf libgtk-4-dev \
    libgtk4-layer-shell-dev gettext
dpkg-buildpackage -b -us -uc        # 加 -d 可跳过构建依赖检查
sudo apt install ../ucabled_*.deb
```

构建需要 cargo 1.92 或更新版本，因为 GTK4 绑定要求如此。若发行版自带的
工具链更旧（Debian trixie、Ubuntu 24.04），改用 rustup 安装的工具链：

```bash
debian/rules binary CARGO=$HOME/.cargo/bin/cargo
```

该包会安装守护进程、两个 helper、systemd 单元、udev 规则、D-Bus 与 polkit
策略、手册页、shell 补全和翻译。它会创建 `ucabled` 服务账户、启动守护进程，
并全局启用每用户会话代理；安装后请重新登录一次，让代理注册。caBLE 身份密钥
位于 `/var/lib/ucabled/identity.key`：`apt remove` 会保留，`apt purge` 会删除
（之后手机需要重新配对）。

## 安装（其他发行版）

在 systemd 发行版上，构建并运行安装脚本：

```bash
cargo build --release
sudo ./install.sh            # --prefix DIR 安装到其他位置
```

它会安装三个二进制、创建 `ucabled` 服务账户、放置 udev、D-Bus、polkit 和
systemd 文件，并启用守护进程和会话代理。`sudo ./install.sh --uninstall`
可撤销；`./install.sh --help` 列出可覆盖的路径。

<details><summary>
如果愿意，也可以手动执行等效步骤：
</summary>

```bash
cargo build --release
sudo install -Dm755 target/release/ucabled    /usr/local/bin/ucabled
sudo install -Dm755 target/release/ucable-agent /usr/local/bin/ucable-agent
sudo install -Dm755 target/release/ucable-agent-helper /usr/local/bin/ucable-agent-helper

# GTK4 helper 的翻译（需要 gettext 的 msgfmt）
for po in po/*.po; do
  lang=$(basename "$po" .po)
  sudo install -d /usr/local/share/locale/$lang/LC_MESSAGES
  sudo msgfmt "$po" -o /usr/local/share/locale/$lang/LC_MESSAGES/ucable-agent-helper-gtk.mo
done

# 专用服务账户、内核模块与 udev 规则
sudo groupadd --system ucabled
sudo useradd --system --gid ucabled --no-create-home ucabled
sudo modprobe uhid
sudo cp dist/90-ucabled.rules /etc/udev/rules.d/
sudo udevadm control --reload

# 系统 D-Bus 策略 + polkit 动作
sudo cp dist/org.ucabled.conf  /etc/dbus-1/system.d/
sudo cp dist/org.ucabled.policy /usr/share/polkit-1/actions/

# 系统守护进程
sudo cp dist/ucabled.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ucabled

# 每用户会话代理
mkdir -p ~/.config/systemd/user
cp dist/ucable-agent.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ucable-agent
```
</details>

重新加载 udev 规则（`sudo udevadm trigger --subsystem-match=misc`）并注销/
登录一次。如果二进制在别处，调整 `ExecStart` 路径。

## 使用

在 Firefox 里正常使用 WebAuthn（注册或登录）。ucabled 会弹出一个小小的置顶
二维码窗口；扫码，在手机上确认（Face ID / 指纹），浏览器随即完成操作。关闭
窗口或点击取消即可中止。

- 窗口由 `ucable-agent` 会话代理显示。如果它没在运行，或者你不是活动的本地
  会话，请求会以 CTAP 超时失败，而不是悄悄开始一次事务。守护进程加
  `--no-ui` 时会把二维码打印到控制终端。
- 取消、超时和断开都会映射回 Firefox 对应的 CTAP 错误码。

### UI helper

`ucable-agent` 本身不绘制任何东西：每次提示时它启动一个 helper，并通过
argv、stdin 和退出状态与之通信。内置的 `ucable-agent-helper` 是一个原生
GTK4 程序：普通情况下是一个带标题栏和关闭按钮的窗口；加 `--layer-shell`
时在 Sway 等 wlroots 合成器上变成 wlr-layer-shell 覆盖层——一张无边框的
圆角卡片，可用 Esc 或由代理关闭。它的界面字符串用 gettext 本地化（Nix 包会
安装这些翻译目录）。用 `--features adwaita` 构建可获得 GNOME 原生窗口外观。

任何符合 contract-v1 的 helper 都能替换它：设置 `UCABLED_HELPER`（绝对路径
或裸命令名），或在 NixOS 上设置 `services.ucabled.helper`。额外的 helper
参数放在 `services.ucabled.helperExtraArgs`（例如 `[ "--layer-shell" ]`）。

### SSH 安全密钥

OpenSSH 的 FIDO `sk` 密钥同样可用，无需浏览器：`ssh` 和 `ssh-keygen` 直接
与虚拟设备通信，因此在活动会话中运行 `ssh-keygen -t ecdsa-sk` 或
`ssh-keygen -Y sign` 会弹出同一个二维码窗口，手机先要求输入密码/生物识别
（用户验证）。

```bash
# 创建密钥。手机没有非驻留模式：每个密钥都是手机上的
# 可发现通行密钥；ssh 用来选中它的 key handle 存放在
# 私钥文件里。
ssh-keygen -t ecdsa-sk -f ~/.ssh/id_ecdsa_sk

# -O resident 会被接受，但在手机上改变不大，因为每个
# 通行密钥在手机上都已是可发现的。
ssh-keygen -t ecdsa-sk -O resident -f ~/.ssh/id_ecdsa_resident

# 把公钥装到服务器上并登录。
ssh-copy-id -i ~/.ssh/id_ecdsa_sk.pub you@example.com
ssh -i ~/.ssh/id_ecdsa_sk you@example.com

# 对文件签名并验证（不涉及 SSH 服务器）。
ssh-keygen -Y sign -f ~/.ssh/id_ecdsa_sk -n file ./file    # 写出 ./file.sig
printf '%s %s\n' "$USER" "$(cat ~/.ssh/id_ecdsa_sk.pub)" > allowed_signers
ssh-keygen -Y verify -f allowed_signers -I "$USER" -n file -s ./file.sig < ./file
```

每一步都会打开二维码窗口；请在活动会话中运行。对手机上已存在的通行密钥重新
登记时，会先询问是否覆盖。

- 用 `ecdsa-sk`。**不支持** `ed25519-sk`：手机上的通行密钥提供方只签
  ES256，因此遇到 Ed25519 凭证时会关闭会话。
- 手机的通行密钥提供方忽略 `rk=false`，总是创建可发现凭证，所以这里没有
  真正的非驻留密钥；`-O resident` 除了登记流程外改变不大。
- **不支持**用 `ssh-keygen -K` 下载驻留密钥：它使用 CTAP 的凭证管理命令，
  而手机的通行密钥提供方不暴露这些命令。请保留登记时写出的私钥文件（或重新
  登记）。
- SSH 客户端必须在本机活动本地会话中运行；无法远程使用，也没有 `ssh-agent`
  集成。

## 安全

- 主机不保留任何密钥材料：私钥和所有 FIDO 操作都留在手机上。守护进程只是
  中继；隧道传输端到端加密的 CTAP（Noise KNpsk0、AES-256-GCM），因此隧道
  服务器只能看到密文，而 BLE 广播是密码学距离证明。
- **`/dev/uhid` 只授予专用的 `ucabled` 服务账户。** 它允许进程创建任意虚拟
  HID 设备（包括键盘），所以不给普通用户；Firefox 只需要随之生成的 hidraw
  节点，该节点仍由 systemd 的 FIDO 规则授予 uaccess。守护进程以非特权身份
  运行在 systemd 沙箱中。如果该节点仍可被其他账户写入（例如残留的 udev
  `uaccess` 标签），守护进程会在启动时记录醒目警告，而不会自行修改权限。
- **只有活动的本地会话才能显示二维码窗口。** 会话代理以 D-Bus 代理身份
  注册，守护进程通过 polkit（`allow_active=yes`）授权，并在每次提示时重新
  检查。SSH 和远程会话在结构上就会被拒绝。
- 事务密钥（在二维码里）以 D-Bus 单播消息从守护进程送到代理，再通过管道
  交给 helper——绝不经过命令行或日志。它是一个短时、一次性的 bearer token：
  在事务结束前请把屏幕视为敏感信息。

## 故障排查

| 现象 | 处理 |
| --- | --- |
| 没有弹出二维码窗口 | `ucable-agent` 未注册：`systemctl --user status ucable-agent`、`journalctl --user -u ucable-agent -f`。只有活动的本地会话才能显示窗口 |
| 浏览器看不到设备 | `ls /dev/hidraw*`；`systemctl status ucabled`；用 `udevadm info` 检查 `ID_FIDO_TOKEN=1` |
| 事务总是失败 | 确认蓝牙已开（系统不会替你打开）；`journalctl -u ucabled -f` |
| 手机扫不了二维码 | 确认日志里没有 `Bluetooth adapter is powered off`，且手机蓝牙正常 |
| Firefox 提示 "Multiple devices found"（插着另一个安全密钥） | 触碰那个安全密钥来使用它，或在 ucabled 窗口中点击 **使用手机**。Firefox 本身只能通过触碰来选定认证器，因此通过 ucabled 自己的提示来选择手机 |

日志只包含命令字节和长度，绝不含原始 CBOR 载荷。

## 已知限制

- **只测试过 iOS**（iCloud 钥匙串、Google 密码管理器和 Strongbox）；Android
  未测试。iOS 不支持状态辅助配对（"记住这台电脑"、免扫码重连），已搁置。
- 只有活动的本地会话能拿到窗口；纯 TTY 或 SSH 提示不在范围内。
- 配合 Firefox 时，RP 永远看到 `transports: ["usb"]`，这是 Firefox Linux
  CTAP 后端里的硬编码。这仅是外观问题。反过来——把这个提示转发给手机——会被
  剥掉：守护进程在中继前会从 `allowList`/`excludeList` 描述符中删除建议性的
  `transports` 字段，这样那些会拒绝未提及 hybrid 的混合凭证的手机会仍能
  匹配到它。
- OpenSSH 的 FIDO 密钥类型中只有 `ecdsa-sk` 可用；`ed25519-sk` 被拒绝，
  因为手机的通行密钥提供方没有实现 Ed25519 算法。
- 没有本地 PIN/UV，也不做证明信任决策——这些全由手机完成。
- 插着其他认证器（例如一个物理安全密钥）时，Firefox 只允许通过触碰物理
  密钥来挑选，因此 ucabled 会显示自己的"使用手机"窗口。加 `--no-ui` 则无法
  参与选择，会自动退让。

## 开发

```bash
nix develop            # 工具链（没有 cargo 就用 nix-shell）
cargo test             # 单元测试 + 本地中继端到端测试
cargo clippy --all-targets
nix build .#ucabled
nix flake check
```

发布（在 `master` 上、dev shell 内）：`admin/release.sh X.Y.Z` 会提升版本、
写入 changelog、提交并创建签名标签 `vX.Y.Z`。推送该标签会触发
`.github/workflows/release.yml`，它检查标签/manifest/changelog 的一致性以及
标签签名，然后以 changelog 对应小节作为发布说明发布 GitHub release。

内置 GTK4 helper 属于默认构建，所以直接 `cargo build` 需要 GTK4 头文件
（以及 wlr-layer-shell 绑定）和 gettext；Nix dev shell 都提供了。加
`--features adwaita` 可获得 GNOME 原生窗口外观——Nix 包保持不依赖
libadwaita，以维持桌面中立。

仓库里的辅助工具（不随包发布）：

- `src/bin/ucabled-spike.rs`：手动单事务实验（`--advert-hex` 跳过 BLE 扫描）
- `src/bin/mock-phone.rs`、`tests/e2e.rs`：模拟手机和本地隧道中继
- `src/bin/hidraw-probe.rs`：内核 HID 路径诊断
- `src/bin/cable-advert-probe.rs`：显示一个提供 BLE 数据通道的 caBLE 二维码，
  并报告手机是否广播了 L2CAP PSM

翻译目录位于 `po/`。新增或修改 helper 字符串后，在 dev shell 里运行
`admin/update-pot.sh` 重新生成 `po/ucable-agent-helper-gtk.pot`，并把新的
msgid 合并进每个目录。helper 是唯一被翻译的组件。

可选的 CTAP 2.3 BLE 数据通道（L2CAP CoC）是实验性的，默认关闭：用
`--features l2cap` 构建并设置 `UCABLED_BLE_CHANNEL=1`。iPhone 不提供它
（已验证）；它面向 Android。

## 许可证

GPL-3.0-or-later；见 `COPYING`。
