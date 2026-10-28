{
  lib,
  rustPlatform,
  makeWrapper,
  pkg-config,
  dbus,
  gtk4,
  gtk4-layer-shell,
  gettext,
  # Set to false to keep the symbol table (e.g. for profiling builds).
  stripBinaries ? true,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "ucabled";
  version = (lib.importTOML ../Cargo.toml).package.version;

  # Only the files the build consumes: the crate, the gettext catalogs
  # compiled in postInstall and the manual pages/completions installed from
  # dist.  A change anywhere else in the repo (tests, docs, site, CI,
  # scripts) leaves this source and thus the output path unchanged, so the
  # Cachix entry an earlier push published stays valid.  The workflow's
  # path filter in .github/workflows/rust.yml is deliberately wider: it
  # also triggers on what the non-package checks consume.
  src = lib.fileset.toSource {
    root = ./..;
    fileset = lib.fileset.unions [
      ../src
      ../Cargo.toml
      ../Cargo.lock
      ../po
      ../dist
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  # libadwaita is deliberately left out here to keep the helper
  # desktop-neutral; build with the "adwaita" feature for GNOME-native chrome.

  # Where the GTK4 helper looks for its gettext catalogs at runtime.
  UCABLED_LOCALEDIR = "${placeholder "out"}/share/locale";

  nativeBuildInputs = [
    makeWrapper
    pkg-config
    gettext
  ];
  buildInputs = [
    dbus
    gtk4
    gtk4-layer-shell
  ];

  # Rust release builds carry no DWARF debug info, so stdenv's default
  # `strip -S` removes nothing and the symbol table ships in the output.
  # Strip all symbols from the installed binaries instead.
  stripAllList = lib.optionals stripBinaries [ "bin" ];

  postInstall = ''
    # Only ship the daemon and its QR helpers; spike/mock/probe are dev tools.
    rm -f $out/bin/ucabled-spike $out/bin/mock-phone $out/bin/hidraw-probe $out/bin/cable-advert-probe
    # GDK loads its backends as modules, so keep the GTK libraries reachable.
    wrapProgram $out/bin/ucable-agent-helper \
      --prefix LD_LIBRARY_PATH : "${
        lib.makeLibraryPath [
          gtk4
          gtk4-layer-shell
        ]
      }"

    # Compile the gettext catalogs for the GTK4 helper.
    for po in po/*.po; do
      lang=$(basename "$po" .po)
      mkdir -p "$out/share/locale/$lang/LC_MESSAGES"
      msgfmt "$po" -o "$out/share/locale/$lang/LC_MESSAGES/ucable.mo"
    done

    # Manual pages and shell completions.
    install -Dm644 dist/man/ucabled.1 "$out/share/man/man1/ucabled.1"
    install -Dm644 dist/man/ucable-agent.1 "$out/share/man/man1/ucable-agent.1"
    install -Dm644 dist/man/ucable-agent-helper.1 "$out/share/man/man1/ucable-agent-helper.1"
    install -Dm644 dist/completions/ucabled.bash "$out/share/bash-completion/completions/ucabled"
    install -Dm644 dist/completions/ucable-agent-helper.bash "$out/share/bash-completion/completions/ucable-agent-helper"
    install -Dm644 dist/completions/_ucabled "$out/share/zsh/site-functions/_ucabled"
    install -Dm644 dist/completions/_ucable-agent-helper "$out/share/zsh/site-functions/_ucable-agent-helper"
    install -Dm644 dist/completions/ucabled.fish "$out/share/fish/vendor_completions.d/ucabled.fish"
    install -Dm644 dist/completions/ucable-agent-helper.fish "$out/share/fish/vendor_completions.d/ucable-agent-helper.fish"
  '';

  meta = {
    description = "Virtual FIDO2 device that relays WebAuthn ceremonies to a phone via caBLE v2 (hybrid transport)";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "ucabled";
  };
})
