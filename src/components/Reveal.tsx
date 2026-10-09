import { useEffect, useRef, type ReactNode } from "react";

export default function Reveal({
  children,
  className = "",
  delay = 0,
}: {
  children: ReactNode;
  className?: string;
  delay?: number;
}) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const show = () => {
      el.style.transitionDelay = `${delay}ms`;
      el.classList.add("on");
    };
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) {
            show();
            io.disconnect();
          }
        }
      },
      { threshold: 0.12 }
    );
    io.observe(el);
    // fallback: never leave content hidden if IO stalls
    const t = setTimeout(show, 1600 + delay);
    return () => {
      io.disconnect();
      clearTimeout(t);
    };
  }, [delay]);

  return (
    <div ref={ref} className={`rv ${className}`}>
      {children}
    </div>
  );
}
