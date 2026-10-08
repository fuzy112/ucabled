# Security Policy

## Scope

ucabled runs a system daemon holding `/dev/uhid` (virtual HID device creation)
and relays WebAuthn ceremonies to a phone over an end-to-end-encrypted caBLE
v2 channel. Issues worth reporting include, but are not limited to:

- a non-active or remote session being able to trigger or observe a prompt;
- the QR transaction secret leaking into logs, command lines, or the D-Bus
  broadcast channel;
- the virtual HID device being usable by processes other than the `ucabled`
  service account;
- weaknesses in the Noise handshake, EID handling, or tunnel validation.

## Reporting a Vulnerability

Please report vulnerabilities privately rather than opening a public issue:

- GitHub: use the **Security** tab → **Report a vulnerability** on
  <https://github.com/fuzy112/ucabled>, or
- email: <i@fuzy.me>

Encrypted mail is welcome; use the OpenPGP key

```
77FF E384 A087 3BDC  F62C B18B 52C4 E557 8A8B 46D1
```

The public key ships in this repository as [`security-reporting.asc`](security-reporting.asc)
and is also published on GitHub at <https://github.com/fuzy112.gpg>. Fetch it
and verify the fingerprint before use:

```bash
gpg --import security-reporting.asc
gpg --fingerprint 77FFE384A0873BDCF62CB18B52C4E5578A8B46D1
```

You should receive an acknowledgement within a few days. Fixes are developed
and released on `master`; only the latest state of the default branch is
supported — there are no maintained release branches.
