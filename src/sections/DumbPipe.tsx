import Section from "../components/Section";
import Reveal from "../components/Reveal";
import { useCopy } from "../lib/i18n";

export default function DumbPipe() {
  const c = useCopy();
  return (
    <Section num="03" id="pipe" title={c.pipe.title}>
      <div className="grid gap-px sm:grid-cols-2" style={{ background: "var(--phos-12)" }}>
        {c.pipe.points.map(([title, body], i) => (
          <Reveal key={title} delay={(i % 2) * 80} className="h-full">
            <div className="h-full bg-black p-6">
              <div className="mb-2 text-sm font-bold" style={{ color: "var(--phos)" }}>
                <span style={{ color: "var(--phos-40)" }}>+ </span>{title}
              </div>
              <p className="text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
                {body}
              </p>
            </div>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
