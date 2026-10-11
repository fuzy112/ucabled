# Changelog

All notable changes to this project are documented here.  The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The QR window now blurs the code in place once the phone is
  detected, instead of switching to a separate status page.
- nixpkgs flake input now points to the official channel tarball
  rather than the github branch.

## [0.1.2] - 2026-10-11

### Fixed

- Sign-in with Firefox on sites that request the legacy U2F `appid`
  extension (e.g. X/Twitter) no longer fails with "no passkeys for this
  website": the daemon declines Firefox's AppID discovery probe so it
  keeps the WebAuthn RP ID the phone stored the passkey under.
- Time out transactions when the phone connects but never answers the
  handshake, instead of waiting until the user cancels.
- Refuse requests arriving mid-transaction instead of leaving the
  channel stuck.
- Erase the EID key (which decrypts the phone's advertisement) from
  memory once it is no longer needed.

## [0.1.1] - 2026-10-09

### Fixed

- OpenSSH `sk` key registrations no longer replace the previously
  stored passkey on synced passkey providers (e.g. iCloud Keychain):
  OpenSSH reuses one fixed user handle for every `rp.id = "ssh:"` key,
  and those providers keep a single passkey per (rpId, userHandle).
  makeCredential requests for `ssh:` now carry a fresh random user
  handle and a distinguishable `ssh:xxxxxxxx` label.

## [0.1.0] - 2026-10-09

Initial release.

[Unreleased]: https://github.com/fuzy112/ucabled/compare/v0.1.2...HEAD
[0.1.2]: https://github.com/fuzy112/ucabled/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/fuzy112/ucabled/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fuzy112/ucabled/releases/tag/v0.1.0
