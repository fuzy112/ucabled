{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.ucabled;
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
    # The daemon creates a virtual HID device; the uaccess tag lets the
    # active seat's user open /dev/uhid. The resulting hidraw node gets
    # uaccess via systemd's 60-fido-id.rules + 70-uaccess.rules.
    #
    # SECURITY: /dev/uhid access lets any process of that user create
    # arbitrary virtual HID devices (including a keyboard), not just this
    # FIDO device. See the note in README.md to narrow it to a group.
    boot.kernelModules = [ "uhid" ];
    services.udev.extraRules = ''KERNEL=="uhid", TAG+="uaccess"'';

    # BLE advert reception is cryptographically mandatory for caBLE
    # (proof of proximity); without bluetoothd every transaction fails.
    # mkDefault so hosts with Bluetooth deliberately off can override.
    hardware.bluetooth.enable = lib.mkDefault true;

    systemd.user.services.ucabled = {
      description = "Phone Passkey Bridge";
      # Order after the session's bluetooth.target (active when an adapter is
      # present). Wants, not Requires: the daemon must still start without
      # Bluetooth — caBLE transactions fail individually in that case.
      wantedBy = [ "default.target" ];
      wants = [ "bluetooth.target" ];
      after = [ "bluetooth.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/ucabled";
        Restart = "on-failure";
        # The daemon parses untrusted network/BLE input and spawns the QR
        # helper, which needs the Wayland socket (XDG_RUNTIME_DIR), /dev/dri
        # for wgpu and /dev/uhid, so no PrivateDevices/DevicePolicy here.
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
        SystemCallArchitectures = "native";
        RestrictAddressFamilies = [
          "AF_UNIX"
          "AF_INET"
          "AF_INET6"
          "AF_BLUETOOTH"
          "AF_NETLINK"
        ];
        UMask = "0077";
        LimitCORE = 0;
      };
    };
  };
}
