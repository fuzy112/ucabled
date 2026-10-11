import { useEffect, useState } from "react";
import { useCopy } from "../lib/i18n";

export default function Boot({ onDone }: { onDone: () => void }) {
  const c = useCopy();
  const lines = c.boot.lines;
  const [text, setText] = useState("");
  const [gone, setGone] = useState(false);

  useEffect(() => {
    const full = lines.join("\n");
    let i = 0;
    let cancelled = false;

    const finish = () => {
      if (cancelled) return;
      setText(full);
      setGone(true);
      setTimeout(onDone, 500);
    };

    const iv = setInterval(() => {
      i += 3 + Math.floor(Math.random() * 5);
      if (i >= full.length) {
        clearInterval(iv);
        finish();
      } else {
        setText(full.slice(0, i));
      }
    }, 18);

    const skip = () => {
      cancelled = true;
      clearInterval(iv);
      setGone(true);
      onDone();
    };
    window.addEventListener("keydown", skip, { once: true });
    window.addEventListener("pointerdown", skip, { once: true });
    return () => {
      cancelled = true;
      clearInterval(iv);
    };
  }, [onDone, lines]);

  return (
    <div id="boot" className={gone ? "done" : ""} aria-hidden>
      <pre
        className="absolute left-5 top-6 sm:left-10 sm:top-10 text-[12px] leading-relaxed whitespace-pre-wrap"
        style={{ color: "var(--phos-70)", textShadow: "0 0 6px rgba(0,244,142,.4)" }}
      >
        {text}
        {!gone && <span className="cursor" />}
      </pre>
      <div className="absolute bottom-6 right-8 text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-25)" }}>
        {c.boot.skip}
      </div>
    </div>
  );
}
