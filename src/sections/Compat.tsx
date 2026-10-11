import Section from "../components/Section";
import Reveal from "../components/Reveal";
import { VERSION_TAG } from "../lib/version";
import { useCopy, type CompatStatus } from "../lib/i18n";

const TAG: Record<CompatStatus, [string, string]> = {
  ok: ["[ ok ]", "var(--phos-hi)"],
  maybe: ["[ ?? ]", "var(--phos-55)"],
  no: ["[ no ]", "var(--phos-40)"],
};

export default function Compat() {
  const c = useCopy();
  return (
    <Section num="06" id="compat" title={c.compat.title}>
      <Reveal>
        <div className="notch p-2 sm:p-4">
          <div className="px-3 py-2 text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
            {c.compat.notePrefix}
            {VERSION_TAG}
            {c.compat.noteSuffix}
          </div>
          {c.compat.rows.map(([st, name, note]) => (
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
