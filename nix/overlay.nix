# Shared by flake.nix (overlays.ucabled) and default.nix (overlay), so
# the two entry points cannot drift apart.
final: _prev: {
  ucabled = final.callPackage ./package.nix { };
  ucable-gnome-agent = final.callPackage ./gnome-agent.nix { };
}
