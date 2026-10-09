{ pkgs ? import <nixpkgs> { } }:

pkgs.mkShell {
  packages = with pkgs; [
    nodejs_22
    # Asset maintenance: recompressing the helper screenshots.
    oxipng
  ];
}
