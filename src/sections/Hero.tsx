import Reveal from "../components/Reveal";

const DIAGRAM = `      ┌─────────────┐
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
  └─────────────────────┘`;

const CHIPS = ["v0.1.0", "GPL-3.0-or-later", "rust", "/dev/uhid", "caBLE v2 · CTAP 2.2 §11.5"];

export default function Hero() {
  return (
    <div id="top" className="relative mx-auto max-w-6xl px-5 sm:px-8 pt-28 sm:pt-36 pb-10">
      <Reveal>
        <p className="text-[12px] sm:text-sm" style={{ color: "var(--phos-55)" }}>
          <span style={{ color: "var(--phos-40)" }}>user@linux:~$</span> systemctl enable --now ucabled
          <span className="cursor" />
        </p>
      </Reveal>

      <Reveal delay={80}>
        <h1 className="display glow-hard mt-6 text-[16.5vw] sm:text-[13vw] lg:text-[10.5rem] uppercase select-none">
          ucabled
        </h1>
      </Reveal>

      <Reveal delay={160}>
        <div className="mt-4 flex flex-wrap items-baseline gap-x-6 gap-y-2">
          <span className="glow text-sm sm:text-lg font-bold uppercase tracking-[0.2em]" style={{ color: "var(--phos)" }}>
            phone passkey bridge
          </span>
          <span className="text-xs sm:text-sm max-w-xl" style={{ color: "var(--phos-55)" }}>
            use your phone (iPhone / Android) as a FIDO2 security key on a Linux desktop.
            no browser changes, no extension, no key material on the machine — the passkey never leaves the phone.
          </span>
        </div>
      </Reveal>

      <Reveal delay={240}>
        <div className="mt-6 flex flex-wrap gap-2">
          {CHIPS.map((c) => (
            <span
              key={c}
              className="border px-2.5 py-1 text-[11px] uppercase tracking-wider"
              style={{ borderColor: "var(--phos-25)", color: "var(--phos-70)", background: "var(--phos-07)" }}
            >
              {c}
            </span>
          ))}
        </div>
      </Reveal>

      <Reveal delay={320}>
        <div className="notch mt-12 p-5 sm:p-8">
          <div className="mb-4 flex items-center justify-between text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
            <span>ucabled :: topology</span>
            <span>fig. 0 — one QR scan away</span>
          </div>
          <pre className="ascii">{DIAGRAM}</pre>
        </div>
      </Reveal>

      <Reveal delay={120}>
        <div className="mt-10 flex flex-wrap gap-4">
          <a
            href="#install"
            className="hov border px-5 py-2.5 text-xs uppercase tracking-widest"
            style={{ borderColor: "var(--phos)", color: "#000", background: "var(--phos)", fontWeight: 700 }}
          >
            &gt; ./install.sh
          </a>
          <a
            href="https://github.com/fuzy112/ucabled"
            target="_blank"
            rel="noreferrer"
            className="hov border px-5 py-2.5 text-xs uppercase tracking-widest"
            style={{ borderColor: "var(--phos-40)", color: "var(--phos-70)" }}
          >
            &gt; git clone ↗
          </a>
        </div>
      </Reveal>
    </div>
  );
}
