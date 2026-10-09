import Section from "../components/Section";
import Reveal from "../components/Reveal";
import imgQr from "../assets/helper-qr.png";
import imgFound from "../assets/helper-phone-detected.png";
import imgSelect from "../assets/helper-select.png";

const SHOTS = [
  {
    src: imgQr,
    fig: "fig. 1",
    caption: "makeCredential prompt — rp: github.com",
    sub: "the qr carries the transaction secret; the passkey glyph sits in the centre patch",
  },
  {
    src: imgFound,
    fig: "fig. 2",
    caption: "ble advert received → “phone detected”",
    sub: "the code blurs out once the phone's proximity advert arrives",
  },
  {
    src: imgSelect,
    fig: "fig. 3",
    caption: "select mode — a security key is also connected",
    sub: "firefox can only pick an authenticator by touch, so ucabled asks itself",
  },
];

export default function Shots() {
  return (
    <Section num="02" id="shots" title="in action">
      <Reveal>
        <p className="mb-8 max-w-3xl text-[13px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
          real captures of the bundled <span style={{ color: "var(--phos)" }}>ucable-agent-helper</span> (gtk4,
          adwaita-dark), rendered under xvfb. the qr below is a structurally valid caBLE v2 code with a dead
          demo tunnel id — point a phone at it and nothing will happen.
        </p>
      </Reveal>
      <div className="grid gap-6 md:grid-cols-3 items-start">
        {SHOTS.map((s, i) => (
          <Reveal key={s.fig} delay={i * 90}>
            <figure className="notch p-4">
              <div className="flex justify-center bg-black p-3">
                <img
                  src={s.src}
                  alt={s.caption}
                  className="max-h-[420px] w-auto"
                  style={{ imageRendering: "auto" }}
                />
              </div>
              <figcaption className="mt-4 border-t pt-3" style={{ borderColor: "var(--phos-12)" }}>
                <div className="text-[11px] font-bold uppercase tracking-wider" style={{ color: "var(--phos)" }}>
                  {s.fig} <span style={{ color: "var(--phos-40)" }}>—</span>{" "}
                  <span style={{ color: "var(--phos-90)" }}>{s.caption}</span>
                </div>
                <div className="mt-1 text-[11.5px] leading-relaxed" style={{ color: "var(--phos-40)" }}>
                  {s.sub}
                </div>
              </figcaption>
            </figure>
          </Reveal>
        ))}
      </div>
    </Section>
  );
}
