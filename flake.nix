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
      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });
    };
}
