import { VERSION_TAG } from "../lib/version";
import { useCopy, useLang } from "../lib/i18n";

export default function Header() {
  const c = useCopy();
  const { lang, setLang } = useLang();

  const links: [string, string][] = [
    [c.nav.how, "#how"],
    [c.nav.shots, "#shots"],
    [c.nav.pipe, "#pipe"],
    [c.nav.install, "#install"],
    [c.nav.ssh, "#ssh"],
    [c.nav.compat, "#compat"],
    [c.nav.limits, "#limits"],
  ];

  const switcher = (
    <div className="flex items-center gap-1.5 uppercase">
      <button
        type="button"
        className="hov"
        onClick={() => setLang("en")}
        style={{ color: lang === "en" ? "var(--phos-hi)" : "var(--phos-40)" }}
      >
        EN
      </button>
      <span style={{ color: "var(--phos-25)" }}>/</span>
      <button
        type="button"
        className="hov"
        onClick={() => setLang("zh")}
        style={{ color: lang === "zh" ? "var(--phos-hi)" : "var(--phos-40)" }}
      >
        中文
      </button>
    </div>
  );

  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b bg-black/90" style={{ borderColor: "var(--phos-12)" }}>
      <div className="mx-auto flex max-w-6xl items-center justify-between px-5 sm:px-8 py-3">
        <a href="#top" className="hov text-[13px] font-extrabold tracking-tight" style={{ color: "var(--phos-hi)" }}>
          ucabled<span style={{ color: "var(--phos-40)" }}>_{VERSION_TAG}</span>
        </a>
        <nav className="hidden md:flex items-center gap-5 text-[11px] uppercase tracking-widest" style={{ color: "var(--phos-55)" }}>
          {links.map(([label, href]) => (
            <a key={href} href={href} className="hov">
              {label}
            </a>
          ))}
          {switcher}
          <a
            href="https://github.com/fuzy112/ucabled"
            target="_blank"
            rel="noreferrer"
            className="hov border px-2.5 py-1"
            style={{ borderColor: "var(--phos-25)", color: "var(--phos-70)" }}
          >
            github ↗
          </a>
        </nav>
        <div className="md:hidden flex items-center gap-4 text-[11px] uppercase tracking-widest" style={{ color: "var(--phos-70)" }}>
          {switcher}
          <a href="https://github.com/fuzy112/ucabled" target="_blank" rel="noreferrer" className="hov">
            github ↗
          </a>
        </div>
      </div>
    </header>
  );
}
