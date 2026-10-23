{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    # Dependency advisory scanner, pinned through flake.lock like the rest of
    # the toolchain (CI runs it as `cargo audit`).
    cargo-audit
    # License, source and ban policy for the same lockfile (CI runs `cargo deny`).
    cargo-deny
    # CI lints every shell entry point with it (admin, install.sh, debian).
    shellcheck
    gcc
    pkg-config
    dbus
    # GTK4 helper build dependencies.
    gtk4
    libadwaita
    gtk4-layer-shell
    gettext
  ];

  # Runtime libraries for the GTK4 helper. GDK needs its toolkit libs and, on
  # NixOS, the GPU drivers live in /run/opengl-driver.
  LD_LIBRARY_PATH =
    "/run/opengl-driver/lib:"
    + pkgs.lib.makeLibraryPath (
      with pkgs;
      [
        gtk4
        libadwaita
        gtk4-layer-shell
      ]
    );
}
