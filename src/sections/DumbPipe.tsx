import Section from "../components/Section";
import Reveal from "../components/Reveal";

const POINTS: [string, string][] = [
  ["zero key material", "the private key and every FIDO operation stay on the phone. the daemon is a relay; there is no FIDO cryptography and no keys at rest on the host."],
  ["e2e encrypted tunnel", "the WSS tunnel carries CTAP encrypted with Noise KNpsk0 + AES-256-GCM. the tunnel server sees only ciphertext. the BLE advert is a cryptographic proximity proof."],
  ["uhid stays locked down", "/dev/uhid can forge arbitrary HID devices (including keyboards), so it is granted only to the dedicated ucabled service account — never to the human user. the daemon runs unprivileged in a systemd sandbox."],
  ["active local session only", "the QR window may only be shown by the active local session. the daemon authorizes the session agent through polkit (allow_active=yes), re-checking on every prompt. ssh and remote sessions are refused by construction."],
  ["secret hygiene", "the transaction secret travels daemon → agent → helper over D-Bus unicast and a pipe — never via a command line or the journal. logs contain only command bytes and lengths, never raw CBOR."],
  ["minimal interference", "pure pass-through except a small set of compatibility answers (getInfo, firefox probe requests, an ios-required rp.name/displayName injection). rpid parsing is read-only, purely to title the QR window."],
];

export default function DumbPipe() {
  return (
    <Section num="03" id="pipe" title="a dumb pipe, on purpose">
      <div className="grid gap-px sm:grid-cols-2" style={{ background: "var(--phos-12)" }}>
        {POINTS.map(([t, b], i) => (
          <Reveal key={t} delay={(i % 2) * 80} className="h-full">
            <div className="h-full bg-black p-6">
              <div className="mb-2 text-sm font-bold" style={{ color: "var(--phos)" }}>
                <span style={{ color: "var(--phos-40)" }}>+ </span>{t}
              </div>
              <p className="text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
                {b}
              </p>
            </div>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
