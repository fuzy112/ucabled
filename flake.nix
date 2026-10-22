{
  description = "Phone Passkey Bridge: virtual FIDO2 device that relays WebAuthn ceremonies to a phone over caBLE v2";

  inputs.nixpkgs.url = "https://channels.nixos.org/nixos-26.05/nixexprs.tar.zst";

  outputs =
    { self, nixpkgs }:
    let
      allSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs allSystems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAllSystems (pkgs: {
        ucabled = pkgs.callPackage ./nix/package.nix { };
        ucable-gnome-agent = pkgs.callPackage ./nix/gnome-agent.nix { };
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.ucabled;
      });
      # `nix run .#deb` builds the Debian packages in rootless podman
      # containers (one per distribution), so the host needs no Debian
      # build dependencies.  admin/release-debs.sh signs and uploads the
      # result at release time.
      apps = forAllSystems (pkgs: {
        deb = {
          type = "app";
          program = "${pkgs.callPackage ./nix/debian-app.nix { }}/bin/ucabled-deb";
        };
      });
      overlays.ucabled = final: _prev: {
        ucabled = final.callPackage ./nix/package.nix { };
        ucable-gnome-agent = final.callPackage ./nix/gnome-agent.nix { };
      };
      overlays.default = self.overlays.ucabled;
      nixosModules.ucabled = import ./nix/module.nix { inherit self; };
      nixosModules.default = self.nixosModules.ucabled;
      # README advertises `nix flake check`; these are what it runs. The
      # package check doubles as the build; clippy reuses the package's
      # vendored dependencies and only swaps the build phase.
      checks = forAllSystems (pkgs: {
        inherit (self.packages.${pkgs.stdenv.hostPlatform.system}) ucabled;
        fmt =
          pkgs.runCommand "ucabled-fmt-check"
            { nativeBuildInputs = [
                pkgs.cargo
                pkgs.rustfmt
              ];
            }
            ''
              cd ${nixpkgs.lib.cleanSource ./.}
              cargo fmt --all --check
              touch $out
            '';
        clippy = self.packages.${pkgs.stdenv.hostPlatform.system}.ucabled.overrideAttrs (old: {
          pname = "ucabled-clippy-check";
          nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
          buildPhase = "cargo clippy --all-targets --locked -- -D warnings";
          installPhase = "touch $out";
          doCheck = false;
        });
        # rustdoc warnings are errors too, so a broken intra-doc link or a
        # malformed code block cannot land unnoticed.
        doc = self.packages.${pkgs.stdenv.hostPlatform.system}.ucabled.overrideAttrs (old: {
          pname = "ucabled-doc-check";
          buildPhase = "RUSTDOCFLAGS='-D warnings' cargo doc --locked --no-deps";
          installPhase = "touch $out";
          doCheck = false;
        });
        # Boot the module for real, so the udev ACL, the polkit action, the
        # daemon's readiness/watchdog handshake and its bus name are all
        # exercised rather than merely evaluated.  The virtual token is then
        # driven end to end over the real hidraw node with python-fido2
        # (Yubico's reference CTAP implementation), the same independent
        # client the Linux-Passkey project uses for its VM tests.
        nixos-module =
          let
            e2e = pkgs.writeText "ucabled-e2e.py" ''
              # No UI agent is registered in the VM, so an interactive
              # ceremony must be refused cleanly; everything the daemon
              # answers locally is checked against the CTAP2 model.
              from fido2.ctap import CtapError
              from fido2.ctap2 import Ctap2
              from fido2.hid import CtapHidDevice

              devices = list(CtapHidDevice.list_devices())
              assert len(devices) == 1, f"expected exactly one FIDO device: {devices!r}"
              dev = devices[0]
              ctap2 = Ctap2(dev)

              # CTAPHID PING round-trips.
              assert dev.ping(b"ucabled") == b"ucabled"

              # authenticatorGetInfo, answered locally by the daemon.
              info = ctap2.get_info()
              assert info.versions == ["FIDO_2_0"], info
              assert info.aaguid == b"ucabled-aaguid01", info
              assert info.extensions == ["credProtect"], info
              assert info.options == {"rk": True, "up": True, "uv": True}, info
              assert info.max_msg_size == 7609, info
              assert info.max_creds_in_list == 64, info
              assert info.max_cred_id_length == 1024, info
              assert info.transports == ["hybrid"], info

              # An interactive makeCredential with no UI agent registered must
              # be refused with a clean status, not hang or wedge the channel.
              try:
                  ctap2.send_cbor(0x01)  # authenticatorMakeCredential, no parameters
                  raise AssertionError("makeCredential unexpectedly succeeded")
              except CtapError as e:
                  assert e.code == 0x05, f"unexpected status {e}"  # CTAP1_ERR_TIMEOUT

              # The channel recovers: getInfo still answers.
              assert ctap2.get_info().versions == ["FIDO_2_0"]

              print("ucabled e2e: ok")
            '';
          in
          pkgs.testers.nixosTest {
            name = "ucabled-module";
            nodes.machine =
              { pkgs, ... }:
              {
                imports = [ self.nixosModules.ucabled ];
                services.ucabled.enable = true;
                environment.systemPackages = [
                  (pkgs.python3.withPackages (ps: [ ps.fido2 ]))
                ];
              };
            testScript = ''
              machine.wait_for_unit("multi-user.target")

              # Dedicated service account and its /dev/uhid ACL.
              machine.succeed("getent passwd ucabled")
              machine.succeed("getent group ucabled")
              machine.succeed("grep -rq 'u:ucabled:rw /dev/uhid' /etc/udev/rules.d/")
              machine.succeed("test -c /dev/uhid")

              # The polkit action ships in the system profile.
              machine.succeed(
                  "test -f /run/current-system/sw/share/polkit-1/actions/org.ucabled.policy"
              )

              # The daemon registers the virtual device, reports readiness
              # (Type=notify) and owns its bus name.
              machine.wait_for_unit("ucabled.service")
              machine.succeed("busctl --system status org.ucabled")

              # End to end over the real hidraw node with an independent
              # CTAP client.  udev creates the node asynchronously after the
              # daemon's readiness notification, so retry until it appears.
              machine.wait_until_succeeds("python3 ${e2e}")
            '';
          };
      });
      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });
    };
}
