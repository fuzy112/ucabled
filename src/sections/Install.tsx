import Section from "../components/Section";
import Reveal from "../components/Reveal";
import CopyButton from "../components/CopyButton";
import { useCopy } from "../lib/i18n";

const NIXOS = `{
  inputs.ucabled.url = "github:fuzy112/ucabled/master";

  # in your NixOS module:
  imports = [ inputs.ucabled.nixosModules.ucabled ];
  services.ucabled.enable = true;
}`;

const OTHER = `cargo build --release
sudo ./install.sh
# options: --prefix DIR  /  --uninstall`;

const CACHIX = `nix.settings = {
  extra-substituters = [ "https://ucabled.cachix.org" ];
  extra-trusted-public-keys = [
    "ucabled.cachix.org-1:iEDAC8e63hzBZ/HKtMibPzl6kvTnPblcFs4+C50XvwU="
  ];
};`;

function Panel({
  label,
  code,
  note,
}: {
  label: string;
  code: string;
  note?: string;
}) {
  return (
    <div className="notch flex flex-col p-5 sm:p-6">
      <div className="mb-4 flex items-center justify-between">
        <span className="text-[10px] uppercase tracking-widest" style={{ color: "var(--phos-40)" }}>
          {label}
        </span>
        <CopyButton text={code} />
      </div>
      <pre className="codeblock">{code}</pre>
      {note && (
        <p className="mt-4 text-[12px] leading-relaxed" style={{ color: "var(--phos-55)" }}>
          {note}
        </p>
      )}
    </div>
  );
}

export default function Install() {
  const c = useCopy();
  return (
    <Section num="04" id="install" title={c.install.title}>
      <Reveal>
        <div className="grid gap-6 lg:grid-cols-2">
          <Panel label={c.install.nixos.label} code={NIXOS} note={c.install.nixos.note} />
          <Panel label={c.install.other.label} code={OTHER} note={c.install.other.note} />
        </div>
      </Reveal>
      <Reveal delay={100}>
        <details className="group mt-6">
          <summary
            className="hov cursor-pointer list-none text-[11px] uppercase tracking-widest"
            style={{ color: "var(--phos-55)" }}
          >
            <span className="group-open:hidden">▸</span>
            <span className="hidden group-open:inline">▾</span> {c.install.cacheToggle}
          </summary>
          <div className="mt-3">
            <Panel label={c.install.cache.label} code={CACHIX} note={c.install.cache.note} />
          </div>
        </details>
      </Reveal>
      <Reveal delay={140}>
        <p className="mt-6 text-[12px]" style={{ color: "var(--phos-40)" }}>
          {c.install.req}
        </p>
      </Reveal>
    </Section>
  );
}
