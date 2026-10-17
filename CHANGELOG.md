# Changelog

All notable changes to this project are documented here.  The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Optimize performance.

### Fixed

- Experimental BLE data channel (`l2cap` feature): frame each caBLE
  message with the 4-byte big-endian length prefix the hybrid
  specification requires, instead of relying on L2CAP message
  boundaries.

## [0.1.3] - 2026-10-13

### Changed

- The QR window now blurs the code in place once the phone is
  detected, instead of switching to a separate status page.
- nixpkgs flake input now points to the official channel tarball
  rather than the GitHub branch.
- Experimental BLE data channel (`l2cap` feature): the L2CAP connect
  now has a timeout and falls back to the WebSocket tunnel when it
  fails or when the negotiated MTU cannot carry the command, instead
  of hanging the transaction.

### Fixed

- Close the device-selection window as soon as the host cancels the
  transaction, instead of leaving it up until its own timeout.

### Security

- Cap tunnel WebSocket frames before allocation, pin the tunnel TLS
  version to 1.3 instead of relying on defaults, and erase the raw
  ECDH outputs once the handshake keys are derived.

## [0.1.2] - 2026-10-11

### Fixed

- Fix sign-in with Firefox on sites that request the legacy U2F
  `appid` extension (e.g. X/Twitter), which failed with "no passkeys
  for this website": the daemon declines Firefox's AppID discovery
  probe so it keeps the WebAuthn RP ID the phone stored the passkey
  under.
- Time out transactions when the phone connects but never answers the
  handshake, instead of waiting until the user cancels.
- Refuse requests arriving mid-transaction instead of leaving the
  channel stuck.

### Security

- Erase the EID key (which decrypts the phone's advertisement) from
  memory once it is no longer needed.

## [0.1.1] - 2026-10-09

### Fixed

- OpenSSH `sk` key registrations no longer replace the previously
  stored passkey on synced passkey providers (e.g. iCloud Keychain):
  OpenSSH reuses one fixed user handle for every `rp.id = "ssh:"` key,
  and those providers keep a single passkey per (rpId, userHandle).
  `makeCredential` requests for `ssh:` now carry a fresh random user
  handle and a distinguishable `ssh:xxxxxxxx` label.

## [0.1.0] - 2026-10-09

Initial release.

[Unreleased]: https://github.com/fuzy112/ucabled/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/fuzy112/ucabled/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/fuzy112/ucabled/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/fuzy112/ucabled/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fuzy112/ucabled/releases/tag/v0.1.0
