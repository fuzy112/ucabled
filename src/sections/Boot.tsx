import { useEffect, useState } from "react";

const LINES = [
  "$ systemctl status ucabled",
  "● ucabled.service — Phone Passkey Bridge",
  "   Active: active (relay) since boot",
  "$ ls -l /dev/uhid && bluetoothctl show | grep Powered",
  "crw-rw----+ 1 root ucabled  uhid",
  "   Powered: yes",
  "> load interface .......... 100%",
];

export default function Boot({ onDone }: { onDone: () => void }) {
  const [text, setText] = useState("");
  const [gone, setGone] = useState(false);

  useEffect(() => {
    const full = LINES.join("\n");
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
  }, [onDone]);

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
        press any key to skip
      </div>
    </div>
  );
}
