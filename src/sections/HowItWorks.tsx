import Section from "../components/Section";
import Reveal from "../components/Reveal";
import { useCopy } from "../lib/i18n";

export default function HowItWorks() {
  const c = useCopy();
  return (
    <Section num="01" id="how" title={c.how.title}>
      <div className="grid gap-px md:grid-cols-3" style={{ background: "var(--phos-12)" }}>
        {c.how.steps.map((s, i) => (
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
