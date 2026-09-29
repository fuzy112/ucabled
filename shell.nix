{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    cargo
    rustc
    rustfmt
    clippy
    gcc
    pkg-config
    dbus
  ];

  # egui/eframe runtime libs (winit Wayland + wgpu/Vulkan). The NixOS hardware
  # drivers (Vulkan ICDs) live in /run/opengl-driver.
  LD_LIBRARY_PATH =
    "/run/opengl-driver/lib:"
    + pkgs.lib.makeLibraryPath (
      with pkgs;
      [
        wayland
        libxkbcommon
        vulkan-loader
      ]
    );
}
