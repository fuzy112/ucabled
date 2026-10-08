{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ucabled;

  # The session agent looks up its UI helper through UCABLED_HELPER: an
  # absolute path (used only if it exists) or a bare command name resolved
  # via PATH. A package is reduced to its main program.
  helperExe =
    if cfg.helper == null then
      # Only reached when extra arguments force an explicit path; without them
      # the agent finds the helper bundled next to its own executable.
      "${cfg.package}/bin/ucable-agent-helper"
    else if lib.isDerivation cfg.helper then
      lib.getExe cfg.helper
    else
      toString cfg.helper;

  # The agent passes arguments of its own (the relying party, --timeout and
  # --select), so extra helper arguments need a launcher in front of it. The
  # agent's arguments come last and win where the two overlap.
  helperLauncher = pkgs.writeShellScript "ucable-agent-helper-launcher" ''
    exec ${helperExe} ${lib.escapeShellArgs cfg.helperExtraArgs} "$@"
  '';

  helperEnv =
    if cfg.helperExtraArgs != [ ] then
      "${helperLauncher}"
    else if cfg.helper == null then
      null
    else
      helperExe;

  # D-Bus system policy: the daemon owns org.ucabled and only needs to call
  # the bus driver, polkit, BlueZ, and the session agents (unique names, so
  # matched by interface). Signals and requested replies are already allowed
  # by the default policy; only method calls are opened here. The content
  # lives in dist/org.ucabled.conf so the manual install and this module
  # cannot drift apart.
  dbusPolicy = pkgs.runCommand "ucabled-dbus-policy" { } ''
    install -Dm644 ${../dist/org.ucabled.conf} \
      $out/share/dbus-1/system.d/org.ucabled.conf
  '';

  # Only the active local session may register an agent. The
  # org.freedesktop.policykit.owner annotation lets the non-root ucabled
  # service call CheckAuthorization on a *session user's* subject: without it
  # polkit rejects cross-uid checks from anyone but uid 0. Single-sourced
  # from dist/org.ucabled.policy, like the D-Bus policy above.
  polkitAction = pkgs.runCommand "ucabled-polkit-action" { } ''
    install -Dm644 ${../dist/org.ucabled.policy} \
      $out/share/polkit-1/actions/org.ucabled.policy
  '';
in
{
  options.services.ucabled = {
    enable = lib.mkEnableOption "Phone Passkey Bridge (virtual FIDO2 device relaying to a phone via caBLE v2)";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.ucabled;
      defaultText = lib.literalExpression "ucabled.packages.\${pkgs.stdenv.hostPlatform.system}.ucabled";
      description = "The ucabled package to use.";
    };

    helper = lib.mkOption {
      type = lib.types.nullOr (
        lib.types.either lib.types.package (lib.types.either lib.types.path lib.types.str)
      );
      default = null;
      example = "\${pkgs.ucable-agent-helper-gnome}/bin/ucable-agent-helper";
      description = ''
        UI helper the per-user session agent spawns to show prompts. May be a
        package (its main program is exported), a path, or a bare command name
        resolved through the agent's `PATH`. When `null`, the agent uses the
        bundled `ucable-agent-helper` binary next to its own executable; that
        is a native GTK4 helper.
      '';
    };

    helperExtraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "--layer-shell" ];
      description = ''
        Extra arguments to start the UI helper with. They are helper-specific:
        the agent adds its own arguments on top (the relying party,
        `--timeout`, `--select`), which win where the two overlap.

        Use this for flags the agent does not know about — for example
        `--layer-shell`, which makes the GTK4 helper a wlr-layer-shell overlay
        on wlroots compositors such as Sway or Hyprland. Helpers that do not
        understand an argument may misread it, so only list flags your helper
        actually takes.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    boot.kernelModules = [ "uhid" ];

    # SECURITY: /dev/uhid lets a process create arbitrary virtual HID devices
    # (including a keyboard). Grant the dedicated service account rw via an
    # ACL, leaving the node's group alone; the human user only needs the
    # resulting hidraw node (uaccess via systemd's FIDO/uaccess rules).
    services.udev.extraRules = ''
      KERNEL=="uhid", RUN+="${pkgs.acl}/bin/setfacl -m u:ucabled:rw /dev/uhid"
    '';

    # BLE advert reception is cryptographically mandatory for caBLE.
    hardware.bluetooth.enable = lib.mkDefault true;

    # Registration authorization goes through polkit.
    security.polkit.enable = true;

    users.groups.ucabled = { };
    users.users.ucabled = {
      isSystemUser = true;
      group = "ucabled";
      description = "Phone Passkey Bridge";
    };

    services.dbus.packages = [ dbusPolicy ];
    environment.systemPackages = [ polkitAction ];

    # BlueZ needs no polkit rule: it registers no polkit actions, and the
    # D-Bus policy above already grants the daemon access to org.bluez.
    systemd.services.ucabled = {
      description = "Phone Passkey Bridge (virtual FIDO2 device relaying to a phone via caBLE v2)";
      # Wants, not Requires: the daemon must still start without Bluetooth.
      wantedBy = [ "multi-user.target" ];
      wants = [ "bluetooth.target" ];
      after = [ "bluetooth.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/ucabled";
        User = "ucabled";
        Group = "ucabled";
        Restart = "on-failure";
        # Persistent caBLE identity key lives in /var/lib/ucabled.
        StateDirectory = "ucabled";
        # The daemon only needs /dev/uhid plus the system bus and the network.
        NoNewPrivileges = true;
        PrivateTmp = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        ProtectProc = "invisible";
        ProcSubset = "pid";
        RestrictNamespaces = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
        SystemCallFilter = [ "@system-service" ];
        CapabilityBoundingSet = [ "" ];
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
          "AF_BLUETOOTH"
          "AF_NETLINK"
        ];
        # /dev/uhid is gated by the ucabled group (mode 0660); a cgroup device
        # filter would be redundant and has path-resolution failure modes, so
        # leave it off.
        UMask = "0077";
        LimitCORE = 0;
        MemorySwapMax = 0;
        MemoryMax = "64M";
      };
    };

    # Per-user UI agent, enabled for every user; polkit decides who may
    # actually register (the active local session only). Bind it to
    # graphical-session.target, not default.target: the latter is started by
    # lingering even with no login, where DISPLAY/WAYLAND_DISPLAY are unset.
    # The compositor starts graphical-session.target only after importing its
    # environment, so the agent sees the display and dies with the session.
    systemd.user.services.ucable-agent = {
      description = "Phone Passkey Bridge UI agent";
      wantedBy = [ "graphical-session.target" ];
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/ucable-agent";
        Restart = "on-failure";
        NoNewPrivileges = true;
        LimitCORE = 0;
        MemorySwapMax = 0;
        Environment = lib.mkIf (helperEnv != null) [ "UCABLED_HELPER=${helperEnv}" ];
      };
    };
  };
}
