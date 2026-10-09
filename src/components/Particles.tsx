import { useEffect, useRef } from "react";

/** Faint phosphor dust drifting upward — CRT ambience layer. */
export default function Particles() {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const cv = ref.current;
    if (!cv) return;
    const ctx = cv.getContext("2d");
    if (!ctx) return;

    let w = (cv.width = window.innerWidth);
    let h = (cv.height = window.innerHeight);
    const onResize = () => {
      w = cv.width = window.innerWidth;
      h = cv.height = window.innerHeight;
    };
    window.addEventListener("resize", onResize);

    const N = Math.min(70, Math.floor((w * h) / 26000));
    const ps = Array.from({ length: N }, () => ({
      x: Math.random() * w,
      y: Math.random() * h,
      v: 0.08 + Math.random() * 0.3,
      s: Math.random() < 0.85 ? 1 : 2,
      a: 0.05 + Math.random() * 0.22,
      tw: Math.random() * Math.PI * 2,
    }));

    let raf = 0;
    let t = 0;
    const tick = () => {
      t += 0.016;
      ctx.clearRect(0, 0, w, h);
      for (const p of ps) {
        p.y -= p.v;
        if (p.y < -4) {
          p.y = h + 4;
          p.x = Math.random() * w;
        }
        const a = p.a * (0.6 + 0.4 * Math.sin(t * 2 + p.tw));
        ctx.fillStyle = `rgba(0,244,142,${a.toFixed(3)})`;
        ctx.fillRect(p.x, p.y, p.s, p.s);
      }
      raf = requestAnimationFrame(tick);
    };
    // rAF already stops in a hidden tab, but be explicit so the canvas
    // never spins while off-screen in a visible-but-backgrounded state.
    const onVisibility = () => {
      cancelAnimationFrame(raf);
      if (!document.hidden) raf = requestAnimationFrame(tick);
    };
    document.addEventListener("visibilitychange", onVisibility);
    raf = requestAnimationFrame(tick);

    return () => {
      cancelAnimationFrame(raf);
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", onResize);
    };
  }, []);

  return (
    <canvas
      ref={ref}
      className="pointer-events-none fixed inset-0 z-0"
      aria-hidden
    />
  );
}
