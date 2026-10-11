import { useCopy } from "../lib/i18n";

export default function Marquee() {
  const c = useCopy();
  const row = (
    <>
      {c.marquee.items.map((item, i) => (
        <span key={i} className="mx-6 inline-flex items-center gap-6">
          <span>{item}</span>
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
