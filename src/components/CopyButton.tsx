import { useState } from "react";

export default function CopyButton({ text }: { text: string }) {
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
      {ok ? "copied ✓" : "copy"}
    </button>
  );
}
