import Reveal from "./Reveal";

export default function Section({
  num,
  id,
  title,
  children,
}: {
  num: string;
  id: string;
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section id={id} className="sec-rule relative mx-auto max-w-6xl px-5 sm:px-8 py-16 sm:py-24">
      <span className="sec-bar absolute -left-[2px] top-16 sm:top-24 h-10 w-[2px]" />
      <Reveal>
        <div className="mb-10 flex items-baseline gap-4">
          <span className="text-xs" style={{ color: "var(--phos-40)" }}>
            [{String(num).padStart(2, "0")}]
          </span>
          <h2 className="glow text-xl sm:text-3xl font-extrabold uppercase tracking-tight" style={{ color: "var(--phos-hi)" }}>
            {title}
          </h2>
        </div>
      </Reveal>
      {children}
    </section>
  );
}
