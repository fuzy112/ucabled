import Section from "../components/Section";
import Reveal from "../components/Reveal";

const LIMITS = [
  "only ios is battle-tested. android speaks the same caBLE v2 protocol but has not been run against real hardware yet.",
  "state-assisted linking (\"remember this computer\", scan-free reconnect) is not supported on ios and is shelved.",
  "only the active local session gets the QR window; a pure tty or ssh prompt is out of scope.",
  "firefox always reports transports: [\"usb\"] to rps — a hardcode in firefox's linux ctap backend. cosmetic only; the daemon strips the advisory hint before relaying so phones still match hybrid credentials.",
  "no local pin/uv and no attestation trust decisions — the phone does all of that.",
  "with another authenticator plugged in, firefox can only pick a key by touch; ucabled shows its own \"use phone\" prompt instead. run with --no-ui and it stands down.",
];

export default function Limits() {
  return (
    <Section num="07" id="limits" title="known limitations">
      <div className="max-w-3xl">
        {LIMITS.map((l, i) => (
          <Reveal key={i} delay={i * 40}>
            <p className="py-2 text-[13px] leading-relaxed" style={{ color: "var(--phos-55)", borderBottom: "1px dashed var(--phos-12)" }}>
              <span style={{ color: "var(--phos)" }}>! </span>
              {l}
            </p>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
