# ucabled — Phone Passkey Bridge

Use your phone (iPhone / Android) as a FIDO2 security key on a Linux desktop:
no Firefox changes, no extension, and no key material on the machine — the
passkey always lives on the phone.

ucabled registers a virtual USB HID FIDO2 device with the kernel
(`/dev/uhid`) and relays WebAuthn registration/sign-in requests from Firefox
to the phone over **caBLE v2** (QR code + BLE proximity proof + WSS tunnel,
the same protocol as Chromium's "use a passkey on your phone"), then sends the
phone's response back to Firefox.

```
Firefox ──CTAP-HID (64B reports)──▶ /dev/uhid ──▶ ucabled        (system service)
                                                    │ caBLE: BLE scan + WSS tunnel
                                                    │ D-Bus: org.ucabled
                                                    ▼
                                          ucable-agent       (session agent)
                                                    │  QR window
                                                    ▼
                                          iPhone (iCloud Keychain)
                                          Android (Google Password Manager)
```

The machine is only a relay: no FIDO cryptography, no keys at rest, and no
business-level CBOR parsing (rpId is read-only, purely to title the QR window).

## Status

- Verified on NixOS + Firefox + iPhone (iCloud Keychain), both registration and
  sign-in.
- Android (Google Password Manager) speaks the same protocol and is expected to
  work, but has not been tested on real hardware.
- Requires Linux (`/dev/uhid`), BlueZ/Bluetooth enabled, and a Firefox that can
  see hidraw. Flatpak/Snap Firefox needs extra udev configuration; the native
  NixOS package works out of the box.

## Install (NixOS, recommended)

Add this repository to your flake and enable the module:

```nix
{
  inputs.ucabled.url = "path:/path/to/ucabled";  # or github:you/ucabled

  # in your NixOS module:
  imports = [ inputs.ucabled.nixosModules.ucabled ];
  services.ucabled.enable = true;
}
```

The module configures everything:

- a dedicated `ucabled` system user and group
- udev rule granting the `ucabled` account rw to `/dev/uhid` via an ACL
  (`RUN+="... setfacl -m u:ucabled:rw /dev/uhid"`), leaving the node's group
  alone
- `boot.kernelModules = [ "uhid" ]`
- `hardware.bluetooth.enable = true` (`mkDefault`, overridable; BLE adverts are
  cryptographically required by caBLE)
- the system service `systemd.services.ucabled`
- the per-user agent `systemd.user.services.ucable-agent`, enabled for every user
- a D-Bus policy, the polkit action `org.ucabled.register-agent`, and a polkit
  rule letting the `ucabled` user drive BlueZ

Run `nixos-rebuild switch`, then log out and back in (or run
`systemctl --user start ucable-agent`) so the agent registers.

## Install (other distros, manual)

```bash
cargo build --release
sudo install -Dm755 target/release/ucabled    /usr/local/bin/ucabled
sudo install -Dm755 target/release/ucable-agent /usr/local/bin/ucable-agent
sudo install -Dm755 target/release/ucable-agent-helper /usr/local/bin/ucable-agent-helper

# dedicated service account, kernel module and udev rule
sudo groupadd --system ucabled
sudo useradd --system --gid ucabled --no-create-home ucabled
sudo modprobe uhid
sudo cp dist/90-ucabled.rules /etc/udev/rules.d/
sudo udevadm control --reload

# system D-Bus policy, polkit action + BlueZ rule
sudo cp dist/org.ucabled.conf  /etc/dbus-1/system.d/
sudo cp dist/org.ucabled.policy /usr/share/polkit-1/actions/
sudo cp dist/50-ucabled-bluez.rules /etc/polkit-1/rules.d/

# system daemon
sudo cp dist/ucabled.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ucabled

# per-user session agent
mkdir -p ~/.config/systemd/user
cp dist/ucable-agent.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ucable-agent
```

Reload the udev rule (`sudo udevadm trigger --subsystem-match=misc`) and log
out/in once. Adjust the `ExecStart` paths if the binaries live elsewhere.

## Usage

Just use WebAuthn in Firefox (register or sign in). ucabled pops up a small
always-on-top window with a QR code; scan it with your phone and follow the
prompt (Face ID / fingerprint), and the browser finishes the operation. Close
the window or click Cancel to abort.

- The window is shown by the `ucable-agent` session agent. If it is not running,
  or you are not the active local session, the request fails with a CTAP
  timeout instead of starting an invisible transaction. Running the daemon with
  `--no-ui` prints the QR code on a controlling terminal instead.
- Cancel/timeout/disconnect are mapped back to Firefox as the proper CTAP
  error codes.

## Security

- The host keeps no key material: the private key and every FIDO operation
  stay on the phone. The daemon is a relay; the tunnel carries end-to-end
  encrypted CTAP (Noise KNpsk0, AES-256-GCM), so the tunnel server sees only
  ciphertext, and the BLE advert is a cryptographic proximity proof.
- **`/dev/uhid` is granted only to the dedicated `ucabled` service account.**
  `/dev/uhid` lets a process create arbitrary virtual HID devices (including a
  keyboard), so it is not given to the human user; Firefox only needs the
  resulting hidraw node, which still gets uaccess from systemd's FIDO rules.
  The daemon runs unprivileged in a systemd sandbox.
- **Only the active local session may show the QR window.** The session agent
  registers as a D-Bus agent, and the daemon authorizes the registration
  through polkit (`allow_active=yes`), re-checking on every prompt. SSH and
  remote sessions are refused by construction.
- The transaction secret (in the QR code) travels from the daemon to the agent
  as a D-Bus unicast message and then to the helper over a pipe — never via a
  command line or the journal. It is a short-lived, single-transaction bearer
  token: treat the screen as sensitive until the transaction ends.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| No QR window appears | The `ucable-agent` agent is not registered: `systemctl --user status ucable-agent` and `journalctl --user -u ucable-agent -f`. Only the active local session may show the window |
| Firefox does not see the device | `ls /dev/hidraw*`; `systemctl status ucabled`; check `udevadm info` for `ID_FIDO_TOKEN=1` |
| Transactions keep failing | Make sure Bluetooth is on (the system will not enable it for you); `journalctl -u ucabled -f` |
| Phone cannot scan the QR code | Make sure the log does not say `Bluetooth adapter is powered off` and that Bluetooth works on the phone |

Logs contain only command bytes and lengths, never raw CBOR payloads.

## Known limitations

- **Only iOS has been tested**; Android is untested. State-assisted linking
  ("remember this computer", scan-free reconnect) is not supported on iOS and
  is shelved; see `docs/linking.md`.
- Only the active local session gets a window; a pure TTY or SSH prompt is out
  of scope.
- RPs always see `transports: ["usb"]`: that is a hardcode in Firefox's Linux
  CTAP backend (see `docs/plan.md` §5.3). It is cosmetic and does not affect
  usage.
- No local PIN/UV and no attestation trust decisions — the phone does all of
  that.

## Development

```bash
nix develop            # toolchain (use nix-shell if you have no cargo)
cargo test             # unit tests + local-relay end-to-end tests
cargo clippy --all-targets
nix build .#ucabled
nix flake check
```

Helper tools in the repo (not shipped in the package):

- `src/bin/ucabled-spike.rs`: manual single-transaction experiment
  (`--advert-hex` skips the BLE scan)
- `src/bin/mock-phone.rs`, `tests/e2e.rs`: mock phone and local tunnel relay
- `src/bin/hidraw-probe.rs`: kernel HID path diagnostics
- `examples/qrgen.rs`: generate a QR code only

Requirements and design: `docs/plan.md`; system service / UI agent design:
`docs/system-service.md`; linking design: `docs/linking.md`.

## License

GPL-3.0-or-later; see `COPYING`.
