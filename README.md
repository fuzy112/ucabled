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
Firefox ──CTAP-HID (64B reports)──▶ /dev/uhid ──▶ ucabled (user service)
                                                    │  QR window + BLE scan + WSS tunnel
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

- `boot.kernelModules = [ "uhid" ]`
- udev rule `KERNEL=="uhid", TAG+="uaccess"` so the logged-in user can open
  `/dev/uhid`
- `hardware.bluetooth.enable = true` (`mkDefault`, overridable; BLE adverts are
  cryptographically required by caBLE)
- `systemd.user.services.ucabled` (starts at login, `Restart=on-failure`)

Run `nixos-rebuild switch` and log out/in once so the uaccess rule takes effect.

## Install (other distros, manual)

```bash
cargo build --release
sudo install -Dm755 target/release/ucabled /usr/local/bin/ucabled
sudo install -Dm755 target/release/ucabled-qr /usr/local/bin/ucabled-qr

# kernel module and udev rule
sudo modprobe uhid
echo 'KERNEL=="uhid", TAG+="uaccess"' | sudo tee /etc/udev/rules.d/90-ucabled.rules
sudo udevadm control --reload

# user service (adjust ExecStart if the binary is installed elsewhere)
mkdir -p ~/.config/systemd/user
cp dist/ucabled.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now ucabled
```

Log out and back in, then it is ready.

## Usage

Just use WebAuthn in Firefox (register or sign in). ucabled pops up a small
always-on-top window with a QR code; scan it with your phone and follow the
prompt (Face ID / fingerprint), and the browser finishes the operation. Close
the window or click Cancel to abort.

- Without a graphical session (or started with `--no-ui`) it falls back to a
  terminal QR code.
- Cancel/timeout/disconnect are mapped back to Firefox as the proper CTAP
  error codes.

## Security

- The host keeps no key material: the private key and every FIDO operation
  stay on the phone. The daemon is a relay; the tunnel carries end-to-end
  encrypted CTAP (Noise KNpsk0, AES-256-GCM), so the tunnel server sees only
  ciphertext, and the BLE advert is a cryptographic proximity proof.
- The transaction secret (in the QR code) is shown on screen and handed to the
  QR helper over a pipe, not via its command line. It is a short-lived,
  single-transaction bearer token: treat the screen as sensitive until the
  transaction ends.
- **`/dev/uhid` access is broad.** The udev rule uses the seat's `uaccess` tag,
  which lets *any* process of the logged-in user create arbitrary virtual HID
  devices — including a virtual keyboard, i.e. input injection. On a machine
  where that user runs untrusted code, this is already a strong capability. To
  narrow it from "whoever holds the active seat" to a chosen account, replace
  the rule with

  ```
  KERNEL=="uhid", GROUP="ucabled", MODE="0660"
  ```

  create the `ucabled` group, and add only the account that runs the daemon to
  it (then re-login or run `udevadm control --reload`).

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| No QR window appears | The user service did not inherit the graphical environment: check `systemctl --user show-environment \| grep WAYLAND`, and if needed run `systemctl --user import-environment WAYLAND_DISPLAY DISPLAY` |
| Firefox does not see the device | `ls /dev/hidraw*`; check `udevadm info` for `ID_FIDO_TOKEN=1`; the uaccess rule needs a re-login |
| Transactions keep failing | Make sure Bluetooth is on (the system will not enable it for you); watch `journalctl --user -u ucabled -f` |
| Phone cannot scan the QR code | Make sure the log does not say `Bluetooth adapter is powered off` and that Bluetooth works on the phone |

Logs contain only command bytes and lengths, never raw CBOR payloads.

## Known limitations

- **Only iOS has been tested**; Android is untested. State-assisted linking
  ("remember this computer", scan-free reconnect) is not supported on iOS and
  is shelved; see `docs/linking.md`.
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

Requirements and design: `docs/plan.md`; linking design: `docs/linking.md`.

## License

MIT
