import Section from "../components/Section";
import Reveal from "../components/Reveal";
import { useCopy } from "../lib/i18n";

export default function Limits() {
  const c = useCopy();
  return (
    <Section num="07" id="limits" title={c.limits.title}>
      <div className="max-w-3xl">
        {c.limits.items.map((item, i) => (
          <Reveal key={i} delay={i * 40}>
            <p className="py-2 text-[13px] leading-relaxed" style={{ color: "var(--phos-55)", borderBottom: "1px dashed var(--phos-12)" }}>
              <span style={{ color: "var(--phos)" }}>! </span>
              {item}
            </p>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
