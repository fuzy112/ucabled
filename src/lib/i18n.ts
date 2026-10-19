import { createContext, useContext } from "react";

export type Lang = "en" | "zh";

export type CompatStatus = "ok" | "maybe" | "no";

export interface Dict {
  meta: { title: string };
  nav: {
    how: string;
    shots: string;
    pipe: string;
    install: string;
    ssh: string;
    compat: string;
    limits: string;
  };
  hero: {
    cmdPrefix: string;
    cmd: string;
    tagline: string;
    subtitle: string;
    fig0: string;
    fig0sub: string;
    installCta: string;
    cloneCta: string;
  };
  how: {
    title: string;
    steps: { tag: string; title: string; body: string; code: string }[];
  };
  shots: {
    title: string;
    intro: string;
    figs: { caption: string; sub: string }[];
  };
  pipe: {
    title: string;
    points: [string, string][];
  };
  install: {
    title: string;
    nixos: { label: string; note: string };
    other: { label: string; note: string };
    cacheToggle: string;
    cache: { label: string; note: string };
    req: string;
  };
  ssh: {
    title: string;
    intro: string;
    label: string;
    note: string;
  };
  compat: {
    title: string;
    notePrefix: string;
    noteSuffix: string;
    rows: [CompatStatus, string, string][];
  };
  limits: { title: string; items: string[] };
  marquee: { items: string[] };
  boot: { lines: string[]; skip: string };
  footer: {
    big: string;
    sub: string;
    github: string;
    releaseSuffix: string;
    license: string;
    stack: string;
    note: string;
  };
  copy: { copy: string; copied: string };
}

export const en: Dict = {
  meta: { title: "ucabled — Phone Passkey Bridge" },
  nav: {
    how: "how",
    shots: "shots",
    pipe: "pipe",
    install: "install",
    ssh: "ssh",
    compat: "compat",
    limits: "limits",
  },
  hero: {
    cmdPrefix: "user@linux:~$",
    cmd: "systemctl enable --now ucabled",
    tagline: "phone passkey bridge",
    subtitle:
      "use your phone (iPhone / Android) as a FIDO2 security key on a Linux desktop. no browser changes, no extension, no keys on the machine — the passkey stays on the phone.",
    fig0: "ucabled :: topology",
    fig0sub: "fig. 0 — one QR scan away",
    installCta: "./install.sh",
    cloneCta: "git clone",
  },
  how: {
    title: "how it works",
    steps: [
      {
        tag: "01 / browser",
        title: "a virtual security key appears",
        body: "ucabled registers a virtual USB HID FIDO2 device with the kernel via /dev/uhid. Firefox sees an ordinary security key — no extension, no patched browser.",
        code: "$ ls /dev/hidraw*\n/dev/hidraw0   # ID_FIDO_TOKEN=1",
      },
      {
        tag: "02 / relay",
        title: "one QR scan, one transaction",
        body: "On a WebAuthn ceremony the session agent shows a small always-on-top QR window. The phone scans it; caBLE v2 (the hybrid transport, now specified by the FIDO Proximity Exchange Protocol) pairs a BLE proximity proof with a WSS tunnel encrypted by Noise KNpsk0 + AES-256-GCM.",
        code: "> scan QR with phone\n> Face ID / fingerprint\n> tunnel: e2e encrypted CTAP",
      },
      {
        tag: "03 / phone",
        title: "the phone does everything",
        body: "The passkey lives on the phone; the phone signs. ucabled relays the assertion back to the browser and tears the tunnel down. The host never holds a key — it is a dumb pipe by design.",
        code: "# host state after sign-in:\n#   private keys: 0\n#   attestation:  phone's own",
      },
    ],
  },
  shots: {
    title: "in action",
    intro:
      "real captures of the bundled «ucable-agent-helper» (gtk4, adwaita-dark), rendered headless. the qr below is a structurally valid caBLE v2 code with a dead demo tunnel id — point a phone at it and nothing will happen.",
    figs: [
      {
        caption: "makeCredential prompt — rp: github.com",
        sub: "the qr carries the transaction secret; the passkey glyph sits in the centre patch",
      },
      {
        caption: "ble advert received → “phone detected”",
        sub: "the code blurs out once the phone's proximity advert arrives",
      },
      {
        caption: "select mode — a security key is also connected",
        sub: "firefox can only pick an authenticator by touch, so ucabled asks itself",
      },
    ],
  },
  pipe: {
    title: "a dumb pipe, on purpose",
    points: [
      [
        "zero key material",
        "the private key and every FIDO operation stay on the phone. the daemon is a relay: no FIDO cryptography, no keys at rest.",
      ],
      [
        "e2e encrypted tunnel",
        "the WSS tunnel carries CTAP encrypted with Noise KNpsk0 + AES-256-GCM. the tunnel server sees only ciphertext. the BLE advert is a cryptographic proximity proof.",
      ],
      [
        "uhid stays locked down",
        "/dev/uhid can forge arbitrary HID devices (including keyboards), so only the dedicated ucabled service account gets it — never the human user. the daemon runs unprivileged in a systemd sandbox.",
      ],
      [
        "active local session only",
        "only the active local session may show the QR window. the daemon authorizes the session agent through polkit (allow_active=yes) and re-checks on every prompt. ssh and remote sessions are refused by construction.",
      ],
      [
        "secret hygiene",
        "the transaction secret travels daemon → agent → helper over D-Bus unicast and a pipe — never via a command line or the journal. logs contain only command bytes and lengths, never raw CBOR.",
      ],
      [
        "minimal interference",
        "pure pass-through except a few compatibility answers (getInfo, firefox probe requests, an ios-required rp.name/displayName injection). rpid parsing is read-only, just to title the QR window.",
      ],
    ],
  },
  install: {
    title: "install",
    nixos: {
      label: "flake.nix — nixos (recommended)",
      note: "the module wires everything: dedicated ucabled user, udev ACL on /dev/uhid, uhid kernel module, bluetooth, systemd system + user services, D-Bus policy and the polkit action. nixos-rebuild switch, then log out and back in.",
    },
    other: {
      label: "any systemd distro",
      note: "installs the three binaries, creates the service account, places the udev, D-Bus, polkit and systemd files, and enables the daemon and session agent.",
    },
    cacheToggle: "binary cache: skip the rust build (cachix)",
    cache: {
      label: "nix.conf — master is published to ucabled.cachix.org",
      note: "CI fills the cache on every push to master. only this repo's artifacts are cached; dependencies still come from cache.nixos.org.",
    },
    req: "requirements: linux with /dev/uhid · bluez/bluetooth on · a browser that can see hidraw (flatpak/snap browsers need extra udev configuration; native packages work out of the box)",
  },
  ssh: {
    title: "ssh security keys too",
    intro:
      "openssh's fido «sk» keys talk to the virtual device directly — no browser needed. each operation pops the same QR window, and the phone asks for its passcode or biometric (user verification) first. works for login and file signing.",
    label: "~/.ssh — each command opens the qr window",
    note: "note: ecdsa-sk only — phone passkey providers don't sign ed25519, and ssh-keygen -K (resident-key download) uses credential-management commands phones don't expose.",
  },
  compat: {
    title: "compatibility",
    notePrefix: "status as of ",
    noteSuffix: " · verified means real hardware, not theory",
    rows: [
      ["ok", "ios — icloud keychain", "registration + sign-in verified on real hardware"],
      ["ok", "ios — google password manager", "registration + sign-in verified"],
      ["ok", "ios — strongbox", "registration + sign-in verified"],
      ["ok", "firefox (native package)", "sees hidraw out of the box; nixos + firefox is the reference setup"],
      ["ok", "openssh ecdsa-sk", "login and file signing; every key is a discoverable passkey on the phone"],
      ["maybe", "android — google password manager", "speaks the same protocol; expected to work, untested on real hardware"],
      ["maybe", "android — ble data channel (l2cap)", "ctap 2.3, experimental, off by default; build --features l2cap"],
      ["no", "ed25519-sk", "phone passkey providers only sign es256"],
      ["no", "ssh-keygen -K", "credential-management commands are not exposed by phones"],
      ["no", "remote / ssh sessions", "refused by construction — active local session only"],
    ],
  },
  limits: {
    title: "known limitations",
    items: [
      "only ios is battle-tested. android speaks the same caBLE v2 protocol but is untested on real hardware.",
      "state-assisted linking (\"remember this computer\", scan-free reconnect) is unsupported on ios and shelved.",
      "only the active local session gets the QR window; a pure tty or ssh prompt is out of scope.",
      "firefox always reports transports: [\"usb\"] to rps — a hardcode in firefox's linux ctap backend. cosmetic only; the daemon strips the advisory hint before relaying so phones still match hybrid credentials.",
      "no local pin/uv and no attestation trust decisions — the phone does all of that.",
      "with another authenticator plugged in, firefox can only pick a key by touch; ucabled shows its own \"use phone\" prompt instead. run with --no-ui and it stands down.",
    ],
  },
  marquee: {
    items: [
      "verified: nixos + firefox + iphone",
      "icloud keychain",
      "google password manager",
      "strongbox",
      "openssh ecdsa-sk",
      "zero key material on host",
      "noise knpsk0 + aes-256-gcm",
    ],
  },
  boot: {
    lines: [
      "$ systemctl status ucabled",
      "● ucabled.service — Phone Passkey Bridge",
      "   Active: active (relay) since boot",
      "$ ls -l /dev/uhid && bluetoothctl show | grep Powered",
      "crw-rw----+ 1 root root 236, 0 /dev/uhid",
      "   Powered: yes",
      "> load interface .......... 100%",
    ],
    skip: "press any key to skip",
  },
  footer: {
    big: "the machine is only a relay.",
    sub: "the passkey stays on the phone.",
    github: "github.com/fuzy112/ucabled ↗",
    releaseSuffix: "— latest release",
    license: "gpl-3.0-or-later",
    stack: "rust · tokio · bluez · gtk4",
    note: "note: this project is entirely generated by large language models (llms). audit before you trust — like anything else holding a pipe to your passkeys.",
  },
  copy: { copy: "copy", copied: "copied ✓" },
};

export const zh: Dict = {
  meta: { title: "ucabled — 手机通行密钥桥" },
  nav: {
    how: "原理",
    shots: "效果",
    pipe: "设计",
    install: "安装",
    ssh: "ssh",
    compat: "兼容",
    limits: "限制",
  },
  hero: {
    cmdPrefix: "user@linux:~$",
    cmd: "systemctl enable --now ucabled",
    tagline: "手机通行密钥桥",
    subtitle:
      "把手机（iPhone / Android）当作 Linux 桌面上的 FIDO2 安全密钥。无需改动浏览器，无需扩展，机器上不存密钥——通行密钥始终留在手机上。",
    fig0: "ucabled :: 拓扑",
    fig0sub: "fig. 0 — 一次扫码之遥",
    installCta: "./install.sh",
    cloneCta: "git clone",
  },
  how: {
    title: "工作原理",
    steps: [
      {
        tag: "01 / 浏览器",
        title: "一个虚拟安全密钥出现",
        body: "ucabled 通过 /dev/uhid 向内核注册虚拟 USB HID FIDO2 设备。Firefox 看到的就是一个普通安全密钥——无需扩展，无需改浏览器。",
        code: "$ ls /dev/hidraw*\n/dev/hidraw0   # ID_FIDO_TOKEN=1",
      },
      {
        tag: "02 / 中继",
        title: "一次扫码，一次事务",
        body: "WebAuthn 流程中，会话代理弹出一个小的置顶二维码窗口。手机扫码；caBLE v2（hybrid transport，现由 FIDO Proximity Exchange Protocol 规范）把 BLE 距离证明与 WSS 隧道配对，隧道由 Noise KNpsk0 + AES-256-GCM 加密。",
        code: "> scan QR with phone\n> Face ID / fingerprint\n> tunnel: e2e encrypted CTAP",
      },
      {
        tag: "03 / 手机",
        title: "一切都由手机完成",
        body: "通行密钥在手机上，签名也由手机完成。ucabled 把断言中继回浏览器并拆掉隧道。主机从不持有密钥——按设计它就是一根哑管道。",
        code: "# host state after sign-in:\n#   private keys: 0\n#   attestation:  phone's own",
      },
    ],
  },
  shots: {
    title: "实际效果",
    intro:
      "内置 «ucable-agent-helper»（gtk4, adwaita-dark）的真实截图，无头渲染。下方二维码是结构有效的 caBLE v2 码，但隧道 id 是死演示值——用手机去扫不会发生任何事。",
    figs: [
      {
        caption: "makeCredential 提示 — rp: github.com",
        sub: "二维码携带事务密钥；钥匙图标位于中心块",
      },
      {
        caption: "收到 BLE 广播 → “检测到手机”",
        sub: "手机的距离广播到达后二维码会模糊",
      },
      {
        caption: "选择模式 — 同时接入了安全密钥",
        sub: "Firefox 只能通过触碰来选定认证器，所以 ucabled 自己询问",
      },
    ],
  },
  pipe: {
    title: "有意为之的哑管道",
    points: [
      [
        "零密钥材料",
        "私钥和所有 FIDO 操作都留在手机上。守护进程只是中继：不做 FIDO 密码学，不在磁盘上保存任何密钥。",
      ],
      [
        "端到端加密隧道",
        "WSS 隧道传输用 Noise KNpsk0 + AES-256-GCM 加密的 CTAP。隧道服务器只能看到密文。BLE 广播是密码学距离证明。",
      ],
      [
        "uhid 保持锁定",
        "/dev/uhid 能伪造任意 HID 设备（包括键盘），所以只给专用的 ucabled 服务账户——绝不给人用户。守护进程以非特权身份运行在 systemd 沙箱中。",
      ],
      [
        "仅限活动的本地会话",
        "只有活动的本地会话才能显示二维码窗口。守护进程通过 polkit（allow_active=yes）授权会话代理，并在每次提示时重新检查。ssh 和远程会话在结构上就会被拒绝。",
      ],
      [
        "密钥卫生",
        "事务密钥经 daemon → agent → helper，走 D-Bus 单播和管道——绝不经过命令行或日志。日志只含命令字节和长度，绝不含原始 CBOR。",
      ],
      [
        "最小干预",
        "纯中继，除少量兼容性应答（getInfo、firefox 探测请求、ios 必需的 rp.name/displayName 注入）外不干预。rpid 解析是只读的，仅用于给二维码窗口取标题。",
      ],
    ],
  },
  install: {
    title: "安装",
    nixos: {
      label: "flake.nix — nixos（推荐）",
      note: "该模块把一切接好：专用 ucabled 用户、/dev/uhid 的 udev ACL、uhid 内核模块、蓝牙、systemd 系统 + 用户服务、D-Bus 策略和 polkit 动作。nixos-rebuild switch，然后注销再登录。",
    },
    other: {
      label: "任意 systemd 发行版",
      note: "安装三个二进制，创建服务账户，放置 udev、D-Bus、polkit 和 systemd 文件，并启用守护进程和会话代理。",
    },
    cacheToggle: "二进制缓存：跳过 rust 构建（cachix）",
    cache: {
      label: "nix.conf — master 发布到 ucabled.cachix.org",
      note: "每次推送到 master 时 CI 填充缓存。只缓存本仓库的产物；依赖仍来自 cache.nixos.org。",
    },
    req: "要求：带 /dev/uhid 的 linux · 开启 bluez/蓝牙 · 能看到 hidraw 的浏览器（flatpak/snap 浏览器需额外的 udev 配置；原生包开箱即用）",
  },
  ssh: {
    title: "也支持 ssh 安全密钥",
    intro:
      "openssh 的 fido «sk» 密钥直接与虚拟设备通信——无需浏览器。每次操作都会弹出同一个二维码窗口，手机先要求输入密码或生物识别（用户验证）。登录和文件签名都适用。",
    label: "~/.ssh — 每个命令都会打开二维码窗口",
    note: "注意：仅 ecdsa-sk——手机的通行密钥提供方不签 ed25519，且 ssh-keygen -K（下载驻留密钥）使用的凭证管理命令手机不暴露。",
  },
  compat: {
    title: "兼容性",
    notePrefix: "状态截至 ",
    noteSuffix: " · 已验证指真机，而非理论",
    rows: [
      ["ok", "ios — icloud 钥匙串", "注册 + 登录已在真机验证"],
      ["ok", "ios — google 密码管理器", "注册 + 登录已验证"],
      ["ok", "ios — strongbox", "注册 + 登录已验证"],
      ["ok", "firefox（原生包）", "开箱即见 hidraw；nixos + firefox 是参考配置"],
      ["ok", "openssh ecdsa-sk", "登录与文件签名；每个密钥都是手机上的可发现通行密钥"],
      ["maybe", "android — google 密码管理器", "使用同一协议，预计可用，但未在真机测试"],
      ["maybe", "android — ble 数据通道 (l2cap)", "ctap 2.3，实验性，默认关闭；用 --features l2cap 构建"],
      ["no", "ed25519-sk", "手机的通行密钥提供方只签 es256"],
      ["no", "ssh-keygen -K", "手机不暴露凭证管理命令"],
      ["no", "远程 / ssh 会话", "结构上被拒——仅限活动的本地会话"],
    ],
  },
  limits: {
    title: "已知限制",
    items: [
      "只对 ios 久经验证。android 使用同一 caBLE v2 协议，但尚未在真机上测试。",
      "ios 不支持状态辅助配对（“记住这台电脑”、免扫码重连），已搁置。",
      "只有活动的本地会话能拿到二维码窗口；纯 tty 或 ssh 提示不在范围内。",
      "firefox 对 rp 永远上报 transports: [\"usb\"]——firefox linux ctap 后端的硬编码。仅是外观问题；守护进程中继前会剥掉这个建议性提示，因此手机仍能匹配 hybrid 凭证。",
      "没有本地 pin/uv，也不做证明信任决策——全由手机完成。",
      "插着其他认证器时，firefox 只能通过触碰选密钥；ucabled 改为显示自己的“使用手机”提示。加 --no-ui 则自动退让。",
    ],
  },
  marquee: {
    items: [
      "已验证：nixos + firefox + iphone",
      "icloud 钥匙串",
      "google 密码管理器",
      "strongbox",
      "openssh ecdsa-sk",
      "主机零密钥材料",
      "noise knpsk0 + aes-256-gcm",
    ],
  },
  boot: {
    lines: [
      "$ systemctl status ucabled",
      "● ucabled.service — 手机通行密钥桥",
      "   Active: active (relay) since boot",
      "$ ls -l /dev/uhid && bluetoothctl show | grep Powered",
      "crw-rw----+ 1 root root 236, 0 /dev/uhid",
      "   Powered: yes",
      "> load interface .......... 100%",
    ],
    skip: "按任意键跳过",
  },
  footer: {
    big: "你的电脑只是中继。",
    sub: "通行密钥始终留在手机上。",
    github: "github.com/fuzy112/ucabled ↗",
    releaseSuffix: "— 最新发布",
    license: "gpl-3.0-or-later",
    stack: "rust · tokio · bluez · gtk4",
    note: "注意：本项目完全由大语言模型（llm）生成。信任前请审计——就像对待任何接入你通行密钥的东西一样。",
  },
  copy: { copy: "复制", copied: "已复制 ✓" },
};

export const DICTS: Record<Lang, Dict> = { en, zh };

export const LangContext = createContext<{
  lang: Lang;
  setLang: (l: Lang) => void;
}>({ lang: "en", setLang: () => {} });

export function useLang() {
  return useContext(LangContext);
}

export function useCopy(): Dict {
  return DICTS[useContext(LangContext).lang];
}
