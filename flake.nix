{
  description = "Phone Passkey Bridge: virtual FIDO2 device that relays WebAuthn ceremonies to a phone over caBLE v2";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";

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
      });
      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });
    };
}
