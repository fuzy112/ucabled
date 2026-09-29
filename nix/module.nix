{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ucabled;

  # D-Bus system policy: the daemon owns org.ucabled and may call the session
  # agent back (to a unique name); local users may call the daemon.
  dbusPolicy = pkgs.writeTextDir "share/dbus-1/system.d/org.ucabled.conf" ''
    <!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
     "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
    <busconfig>
      <policy user="ucabled">
        <allow own="org.ucabled"/>
        <allow send_destination="*"/>
        <allow receive_sender="*"/>
      </policy>
      <policy context="default">
        <allow send_destination="org.ucabled"/>
      </policy>
    </busconfig>
  '';

  # Only the active local session may register a prompter. The
  # org.freedesktop.policykit.owner annotation lets the non-root ucabled
  # service call CheckAuthorization on a *session user's* subject: without it
  # polkit rejects cross-uid checks from anyone but uid 0.
  polkitAction = pkgs.writeTextDir "share/polkit-1/actions/org.ucabled.policy" ''
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE policyconfig PUBLIC
     "-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN"
     "http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd">
    <policyconfig>
      <vendor>ucabled</vendor>
      <action id="org.ucabled.register-prompter">
        <description>Register the phone passkey dialog</description>
        <message>Authentication is required to show passkey prompts</message>
        <defaults>
          <allow_any>no</allow_any>
          <allow_inactive>no</allow_inactive>
          <allow_active>yes</allow_active>
        </defaults>
        <annotate key="org.freedesktop.policykit.owner">unix-user:ucabled</annotate>
      </action>
    </policyconfig>
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

    # Let the daemon drive the Bluetooth adapter even though it is not part of
    # an active session.
    security.polkit.extraConfig = ''
      polkit.addRule(function(action, subject) {
        if (action.id.indexOf("org.bluez.") === 0 && subject.user === "ucabled") {
          return polkit.Result.YES;
        }
      });
    '';

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
      };
    };

    # Per-user UI agent, enabled for every user; polkit decides who may
    # actually register (the active local session only).
    systemd.user.services.ucabled-ui = {
      description = "Phone Passkey Bridge UI agent";
      wantedBy = [ "default.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/ucabled-ui";
        Restart = "on-failure";
        NoNewPrivileges = true;
      };
    };
  };
}
