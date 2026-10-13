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
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.ucabled;
      });
      overlays.ucabled = final: _prev: final.callPackage ./nix/package.nix { };
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
        # exercised rather than merely evaluated.
        nixos-module = pkgs.testers.nixosTest {
          name = "ucabled-module";
          nodes.machine = {
            imports = [ self.nixosModules.ucabled ];
            services.ucabled.enable = true;
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
          '';
        };
      });
      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });
    };
}
