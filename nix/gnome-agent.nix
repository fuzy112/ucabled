{
  lib,
  stdenvNoCC,
  gettext,
}:

# The GNOME Shell agent extension: pure GJS, nothing to build but the
# gettext catalogs.  Version tracks the daemon it talks to.
stdenvNoCC.mkDerivation {
  pname = "ucable-gnome-agent";
  version = (lib.importTOML ../Cargo.toml).package.version;

  src = ../contrib/ucable-gnome-agent;

  dontBuild = true;

  nativeBuildInputs = [ gettext ];

  installPhase = ''
    runHook preInstall
    extdir=$out/share/gnome-shell/extensions/ucable-gnome-agent@fuzy.me
    install -Dm644 metadata.json extension.js qrcode.mjs -t "$extdir"
    # Translations are shared with the GTK helper (gettext domain "ucable");
    # gnome-shell looks them up in the extension's own locale/ directory
    # (Extension.initTranslations()).
    for po in ${../po}/*.po; do
      lang=''${po##*/}
      lang=''${lang%.po}
      install -d "$extdir/locale/$lang/LC_MESSAGES"
      msgfmt "$po" -o "$extdir/locale/$lang/LC_MESSAGES/ucable.mo"
    done
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
