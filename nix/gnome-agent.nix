{
  lib,
  stdenvNoCC,
}:

# The GNOME Shell agent extension: pure GJS, nothing to build. Version
# tracks the daemon it talks to.
stdenvNoCC.mkDerivation {
  pname = "ucabled-gnome-agent";
  version = (lib.importTOML ../Cargo.toml).package.version;

  src = ../contrib/ucabled-gnome-agent;

  dontBuild = true;

  installPhase = ''
    runHook preInstall
    install -Dm644 metadata.json extension.js qrcode.mjs \
      -t $out/share/gnome-shell/extensions/ucabled-agent@fuzy.me
    runHook postInstall
  '';

  meta = {
    description = "GNOME Shell extension acting as the ucabled session UI agent";
    homepage = "https://github.com/fuzy112/ucabled";
    # extension.js is GPL-3.0-or-later; the vendored qrcode.mjs is MIT.
    license = [
      lib.licenses.gpl3Plus
      lib.licenses.mit
    ];
    platforms = lib.platforms.linux;
  };
}
