{
  lib,
  rustPlatform,
  makeWrapper,
  pkg-config,
  dbus,
  vulkan-loader,
  wayland,
  libxkbcommon,
}:

rustPlatform.buildRustPackage (finalAttrs: {
  pname = "ucabled";
  version = "0.1.0";

  src = lib.cleanSource ./..;
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [
    makeWrapper
    pkg-config
  ];
  buildInputs = [ dbus ];

  postInstall = ''
    # Only ship the daemon and its QR helper; spike/mock/probe are dev tools.
    rm -f $out/bin/ucabled-spike $out/bin/mock-phone $out/bin/hidraw-probe
    wrapProgram $out/bin/ucabled-qr \
      --prefix LD_LIBRARY_PATH : "/run/opengl-driver/lib:${
        lib.makeLibraryPath [
          vulkan-loader
          wayland
          libxkbcommon
        ]
      }"
  '';

  meta = {
    description = "Virtual FIDO2 device that relays WebAuthn ceremonies to a phone via caBLE v2 (hybrid transport)";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "ucabled";
  };
})
