# Stable (non-flake) Nix entry point: the packages, overlay and NixOS
# module, with nixpkgs pinned through flake.lock (see nix/nixpkgs.nix).
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
  # With pkgs passed: only the module's packages.<system> lookup key.
  system ? builtins.currentSystem,
  # Build with this package set instead of the pinned nixpkgs.  The
  # default is only forced when a package value is.
  pkgs ? import (import ./nix/nixpkgs.nix) { inherit system; },
}:

let
  packages = {
    ucabled = pkgs.callPackage ./nix/package.nix { };
    ucable-gnome-agent = pkgs.callPackage ./nix/gnome-agent.nix { };
  };
in

packages
// {
  overlay = import ./nix/overlay.nix;

  # nix/module.nix takes the flake's `self`; hand it a shim with the
  # same packages.<system> shape so it needs no flake of its own.
  nixosModule = import ./nix/module.nix {
    self = {
      packages.${system} = packages;
    };
  };
}
