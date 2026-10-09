import { useState } from "react";
import Section from "../components/Section";
import Reveal from "../components/Reveal";
import CopyButton from "../components/CopyButton";

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
  const [showCache, setShowCache] = useState(false);
  return (
    <Section num="04" id="install" title="install">
      <Reveal>
        <div className="grid gap-6 lg:grid-cols-2">
          <Panel
            label="flake.nix — nixos (recommended)"
            code={NIXOS}
            note="the module wires everything: dedicated ucabled user, udev ACL on /dev/uhid, uhid kernel module, bluetooth, systemd system + user services, D-Bus policy and the polkit action. nixos-rebuild switch, then log out and back in."
          />
          <Panel
            label="any systemd distro"
            code={OTHER}
            note="installs the three binaries (ucabled, ucable-agent, ucable-agent-helper), creates the service account and places the udev, D-Bus, polkit and systemd files, then enables the daemon and the session agent."
          />
        </div>
      </Reveal>
      <Reveal delay={100}>
        <div className="mt-6">
          <button
            onClick={() => setShowCache((v) => !v)}
            className="hov text-[11px] uppercase tracking-widest"
            style={{ color: "var(--phos-55)" }}
          >
            {showCache ? "▾" : "▸"} binary cache: skip the rust build (cachix)
          </button>
          {showCache && (
            <div className="mt-3">
              <Panel
                label="nix.conf — master is published to ucabled.cachix.org"
                code={CACHIX}
                note="CI fills the cache on every push to master. only this repo's artifacts are cached; dependencies still come from cache.nixos.org."
              />
            </div>
          )}
        </div>
      </Reveal>
      <Reveal delay={140}>
        <p className="mt-6 text-[12px]" style={{ color: "var(--phos-40)" }}>
          requirements: linux with /dev/uhid · bluez/bluetooth on · a browser that can see hidraw
          (flatpak/snap browsers need extra udev configuration; native packages work out of the box)
        </p>
      </Reveal>
    </Section>
  );
}
