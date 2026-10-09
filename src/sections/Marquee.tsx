const ITEMS = [
  "verified: nixos + firefox + iphone",
  "icloud keychain",
  "google password manager",
  "strongbox",
  "openssh ecdsa-sk",
  "zero key material on host",
  "noise knpsk0 + aes-256-gcm",
];

export default function Marquee() {
  const row = (
    <>
      {ITEMS.map((it, i) => (
        <span key={i} className="mx-6 inline-flex items-center gap-6">
          <span>{it}</span>
          <span style={{ color: "var(--phos-25)" }}>▚</span>
        </span>
      ))}
    </>
  );
  return (
    <div
      className="overflow-hidden border-y py-2.5 text-[11px] uppercase tracking-[0.25em]"
      style={{ borderColor: "var(--phos-12)", color: "var(--phos-55)" }}
      aria-hidden
    >
      <div className="marquee-track">
        <span className="flex items-center">{row}</span>
        <span className="flex items-center">{row}</span>
      </div>
    </div>
  );
}
