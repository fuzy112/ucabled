{
  lib,
  rustPlatform,
  makeWrapper,
  pkg-config,
  dbus,
  gtk4,
  gtk4-layer-shell,
  gettext,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "ucabled";
  version = "0.1.0";

  src = lib.cleanSource ./..;
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

  postInstall = ''
    # Only ship the daemon and its QR helpers; spike/mock/probe are dev tools.
    rm -f $out/bin/ucabled-spike $out/bin/mock-phone $out/bin/hidraw-probe $out/bin/cable-advert-probe
    # GDK loads its backends as modules, so keep the GTK libraries reachable.
    wrapProgram $out/bin/ucable-agent-helper \
      --prefix LD_LIBRARY_PATH : "${lib.makeLibraryPath [ gtk4 gtk4-layer-shell ]}"

    # Compile the gettext catalogs for the GTK4 helper.
    for po in po/*.po; do
      lang=$(basename "$po" .po)
      mkdir -p "$out/share/locale/$lang/LC_MESSAGES"
      msgfmt "$po" -o "$out/share/locale/$lang/LC_MESSAGES/ucable-agent-helper-gtk.mo"
    done
  '';

  meta = {
    description = "Virtual FIDO2 device that relays WebAuthn ceremonies to a phone via caBLE v2 (hybrid transport)";
    license = lib.licenses.gpl3Plus;
    platforms = lib.platforms.linux;
    mainProgram = "ucabled";
  };
})
