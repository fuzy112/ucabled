import Section from "../components/Section";
import Reveal from "../components/Reveal";
import Rich from "../components/Rich";
import { useCopy } from "../lib/i18n";
import imgQr from "../assets/helper-qr.png";
import imgFound from "../assets/helper-phone-detected.png";
import imgSelect from "../assets/helper-select.png";

const SHOTS = [
  { src: imgQr, fig: "fig. 1", w: 394, h: 574 },
  { src: imgFound, fig: "fig. 2", w: 394, h: 574 },
  { src: imgSelect, fig: "fig. 3", w: 414, h: 274 },
];

export default function Shots() {
  const c = useCopy();
  return (
    <Section num="02" id="shots" title={c.shots.title}>
      <Reveal>
        <p className="mb-8 max-w-3xl text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
          <Rich text={c.shots.intro} />
        </p>
      </Reveal>
      <div className="grid gap-6 md:grid-cols-3 items-start">
        {SHOTS.map((s, i) => (
          <Reveal key={s.fig} delay={i * 90}>
            <figure className="notch p-4">
              <div className="flex justify-center bg-black p-3">
                <img
                  src={s.src}
                  alt={c.shots.figs[i].caption}
                  width={s.w}
                  height={s.h}
                  loading="lazy"
                  decoding="async"
                  className="max-h-[420px] w-auto"
                />
              </div>
              <figcaption className="mt-4 border-t pt-3" style={{ borderColor: "var(--phos-12)" }}>
                <div className="text-[11px] font-bold uppercase tracking-wider" style={{ color: "var(--phos)" }}>
                  {s.fig} <span style={{ color: "var(--phos-40)" }}>—</span>{" "}
                  <span style={{ color: "var(--phos-90)" }}>{c.shots.figs[i].caption}</span>
                </div>
                <div className="mt-1 text-[11.5px] leading-relaxed" style={{ color: "var(--phos-40)" }}>
                  {c.shots.figs[i].sub}
                </div>
              </figcaption>
            </figure>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
