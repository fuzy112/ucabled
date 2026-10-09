import { VERSION_TAG } from "../lib/version";

const LINKS = [
  ["how", "#how"],
  ["shots", "#shots"],
  ["pipe", "#pipe"],
  ["install", "#install"],
  ["ssh", "#ssh"],
  ["compat", "#compat"],
  ["limits", "#limits"],
] as const;

export default function Header() {
  return (
    <header className="fixed inset-x-0 top-0 z-50 border-b bg-black/90" style={{ borderColor: "var(--phos-12)" }}>
      <div className="mx-auto flex max-w-6xl items-center justify-between px-5 sm:px-8 py-3">
        <a href="#top" className="hov text-[13px] font-extrabold tracking-tight" style={{ color: "var(--phos-hi)" }}>
          ucabled<span style={{ color: "var(--phos-40)" }}>_{VERSION_TAG}</span>
        </a>
        <nav className="hidden md:flex items-center gap-5 text-[11px] uppercase tracking-widest" style={{ color: "var(--phos-55)" }}>
          {LINKS.map(([label, href]) => (
            <a key={href} href={href} className="hov">
              {label}
            </a>
          ))}
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
        <a
          href="https://github.com/fuzy112/ucabled"
          target="_blank"
          rel="noreferrer"
          className="md:hidden text-[11px] uppercase tracking-widest"
          style={{ color: "var(--phos-70)" }}
        >
          github ↗
        </a>
      </div>
    </header>
  );
}
