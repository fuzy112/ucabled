# Stable (non-flake) Nix entry point: everything flake.nix offers that
# makes sense without flakes, with nixpkgs pinned through flake.lock
# (see nix/nixpkgs.nix).
#
#   nix-build -A ucabled
#   nix-build -A ucable-gnome-agent
#   nix-shell                      # same development shell as nix develop
#
# NixOS configuration without flakes:
#
#   imports = [
#     (import (builtins.fetchTarball
#       "https://github.com/fuzy112/ucabled/archive/refs/heads/master.tar.gz")
#       { }).nixosModule
#   ];
#   services.ucabled.enable = true;
{
  system ? builtins.currentSystem,
  pkgs ? import (import ./nix/nixpkgs.nix) { inherit system; },
}:

let
  lib = pkgs.lib;

  # Keep in step with flake.nix's allSystems.
  allSystems = [
    "x86_64-linux"
    "aarch64-linux"
  ];

  # Per-system package sets, evaluated lazily: the NixOS module picks its
  # target system out of this attrset without instantiating the others.
  packages = lib.genAttrs allSystems (
    s:
    let
      p = import (import ./nix/nixpkgs.nix) { system = s; };
    in
    {
      ucabled = p.callPackage ./nix/package.nix { };
      ucable-gnome-agent = p.callPackage ./nix/gnome-agent.nix { };
    }
  );
in

# The requested system's packages at the top level, so plain `nix-build`
# and `nix-env -iA ucabled -f .` work.
packages.${system}
// {
  inherit packages;

  overlay = import ./nix/overlay.nix;

  # nix/module.nix takes the flake's `self`; hand it a shim with the
  # same packages.<system> shape so it needs no flake of its own.
  nixosModule = import ./nix/module.nix {
    self = { inherit packages; };
  };
}
