import Reveal from "../components/Reveal";
import { RELEASE_URL, VERSION_TAG } from "../lib/version";
import { useCopy } from "../lib/i18n";

export default function Footer() {
  const c = useCopy();
  return (
    <footer className="border-t" style={{ borderColor: "var(--phos-12)" }}>
      <div className="mx-auto max-w-6xl px-5 sm:px-8 py-14">
        <Reveal>
          <p className="glow text-lg sm:text-2xl font-extrabold uppercase" style={{ color: "var(--phos-hi)" }}>
            {c.footer.big}
          </p>
          <p className="mt-2 text-[13px]" style={{ color: "var(--phos-55)" }}>
            {c.footer.sub}
          </p>
        </Reveal>
        <Reveal delay={100}>
          <div className="mt-10 flex flex-wrap items-center gap-x-8 gap-y-3 text-[11px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
            <a className="hov" href="https://github.com/fuzy112/ucabled" target="_blank" rel="noreferrer">
              {c.footer.github}
            </a>
            <a className="hov" href={RELEASE_URL} target="_blank" rel="noreferrer">
              {VERSION_TAG} {c.footer.releaseSuffix}
            </a>
            <span>{c.footer.license}</span>
            <span>{c.footer.stack}</span>
          </div>
          <p className="mt-6 text-[11px]" style={{ color: "var(--phos-25)" }}>
            {c.footer.note}
          </p>
        </Reveal>
      </div>
    </footer>
  );
}
