import Section from "../components/Section";
import Reveal from "../components/Reveal";

type Status = "ok" | "maybe" | "no";
const ROWS: [Status, string, string][] = [
  ["ok", "ios — icloud keychain", "registration + sign-in verified on real hardware"],
  ["ok", "ios — google password manager", "registration + sign-in verified"],
  ["ok", "ios — strongbox", "registration + sign-in verified"],
  ["ok", "firefox (native package)", "sees hidraw out of the box; nixos + firefox is the reference setup"],
  ["ok", "openssh ecdsa-sk", "resident + non-resident keys, login and file signing"],
  ["maybe", "android — google password manager", "speaks the same protocol; expected to work, untested on real hardware"],
  ["maybe", "android — ble data channel (l2cap)", "ctap 2.3, experimental, off by default; build --features l2cap"],
  ["no", "ed25519-sk", "phone passkey providers only sign es256"],
  ["no", "ssh-keygen -K", "credential-management commands are not exposed by phones"],
  ["no", "remote / ssh sessions", "refused by construction — active local session only"],
];

const TAG: Record<Status, [string, string]> = {
  ok: ["[ ok ]", "var(--phos-hi)"],
  maybe: ["[ ?? ]", "var(--phos-55)"],
  no: ["[ no ]", "var(--phos-40)"],
};

export default function Compat() {
  return (
    <Section num="06" id="compat" title="compatibility">
      <Reveal>
        <div className="notch p-2 sm:p-4">
          <div className="px-3 py-2 text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
            status as of v0.1.0 · verified means real hardware, not theory
          </div>
          {ROWS.map(([st, name, note]) => (
            <div key={name} className="trow grid grid-cols-[64px_1fr] sm:grid-cols-[80px_minmax(180px,1fr)_1.2fr] items-baseline gap-x-4 px-3 py-2.5 text-[12.5px]">
              <span className="font-bold" style={{ color: TAG[st][1] }}>
                {TAG[st][0]}
              </span>
              <span className="font-bold" style={{ color: st === "no" ? "var(--phos-40)" : "var(--phos-90)" }}>
                {name}
              </span>
              <span className="col-span-2 sm:col-span-1 mt-0.5 sm:mt-0 text-[12px]" style={{ color: "var(--phos-40)" }}>
                {note}
              </span>
            </div>
          ))}
        </div>
      </Reveal>
    </Section>
  );
}
