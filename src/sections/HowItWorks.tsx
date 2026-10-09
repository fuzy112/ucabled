import Section from "../components/Section";
import Reveal from "../components/Reveal";

const STEPS = [
  {
    tag: "01 / browser",
    title: "a virtual security key appears",
    body: "ucabled registers a virtual USB HID FIDO2 device with the kernel via /dev/uhid. Firefox sees an ordinary security key — no extension, no patched browser, no flags.",
    code: "$ ls /dev/hidraw*\n/dev/hidraw0   # ID_FIDO_TOKEN=1",
  },
  {
    tag: "02 / relay",
    title: "one QR scan, one transaction",
    body: "On a WebAuthn ceremony the session agent pops a small always-on-top QR window. The phone scans it; caBLE v2 (the CTAP 2.2 §11.5 hybrid transport) pairs BLE proximity proof with a WSS tunnel end-to-end encrypted by Noise KNpsk0 + AES-256-GCM.",
    code: "> scan QR with phone\n> Face ID / fingerprint\n> tunnel: e2e encrypted CTAP",
  },
  {
    tag: "03 / phone",
    title: "the phone does everything",
    body: "The passkey lives on the phone; the phone signs. ucabled relays the assertion back to the browser and tears the tunnel down. The host never holds a key — it is a dumb pipe by design.",
    code: "# host state after sign-in:\n#   private keys: 0\n#   attestation:  phone's own",
  },
];

export default function HowItWorks() {
  return (
    <Section num="01" id="how" title="how it works">
      <div className="grid gap-px md:grid-cols-3" style={{ background: "var(--phos-12)" }}>
        {STEPS.map((s, i) => (
          <Reveal key={s.tag} delay={i * 90} className="h-full">
            <div className="flex h-full flex-col gap-4 bg-black p-6">
              <span className="text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
                {s.tag}
              </span>
              <h3 className="glow text-base font-bold uppercase" style={{ color: "var(--phos-hi)" }}>
                {s.title}
              </h3>
              <p className="text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
                {s.body}
              </p>
              <pre className="codeblock mt-auto border p-3" style={{ borderColor: "var(--phos-12)" }}>
                {s.code}
              </pre>
            </div>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
