import { useState } from "react";
import { useCopy } from "../lib/i18n";

export default function CopyButton({ text }: { text: string }) {
  const c = useCopy();
  const [ok, setOk] = useState(false);
  return (
    <button
      onClick={() => {
        navigator.clipboard?.writeText(text).catch(() => {});
        setOk(true);
        setTimeout(() => setOk(false), 1400);
      }}
      className="hov shrink-0 border px-3 py-1 text-[11px] uppercase tracking-widest"
      style={{ borderColor: "var(--phos-25)", color: ok ? "#000" : "var(--phos-70)", background: ok ? "var(--phos)" : "transparent" }}
    >
      {ok ? c.copy.copied : c.copy.copy}
    </button>
  );
}
