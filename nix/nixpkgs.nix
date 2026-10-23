# The nixpkgs pin for stable (non-flake) Nix, single-sourced from
# flake.lock: plain `nix-build`/`nix-shell` users build with the exact
# same nixpkgs as flake users, so the Cachix binary cache applies to
# both.  The lock entry is a tarball input, and builtins.fetchTarball
# verifies it against the recorded NAR hash.  Update the pin with
# `nix flake update` (or `nix flake lock --update-input nixpkgs`),
# never by editing this file.
let
  lock = builtins.fromJSON (builtins.readFile ../flake.lock);
  inherit (lock.nodes.nixpkgs.locked) url narHash;
in
builtins.fetchTarball {
  inherit url;
  sha256 = narHash;
}
