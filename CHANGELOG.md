# Changelog

All notable changes to this project are documented here.  The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project adheres to [Semantic Versioning](https://semver.org/).

## [0.4.0-pre]

### Fixed

- With `--layer-shell` (or `UCABLED_HELPER_LAYER_SHELL=1`), Esc now closes
  the helper window.  The overlay holds the keyboard while it is up, so
  compositor shortcuts stay out until the card goes away; previously it
  never asked for focus, the key went elsewhere, and only finishing or
  abandoning the request could take the window away.

## [0.3.0] - 2026-10-27

### Added

- GitHub releases now carry a source tarball (`ucabled-X.Y.Z.tar.zst`)
  archived from the signed tag, with a detached GPG signature, alongside
  the Debian/Ubuntu packages.  GitHub's automatic "Source code" archives
  are regenerated on demand and stay unsigned.

### Changed

- When Firefox asks which of several authenticators to use ("Multiple
  devices found"), the QR code now appears right away instead of a
  "use the phone instead?" question first: scan the code to pick the
  phone, or touch the other security key (the window says so) to pick
  that one.  The phone is only marked as selected once it has connected,
  so touching the key before that still wins.  The UI helper contract is
  now v3 and has no `--select` mode (see `docs/helper-contract.md`).

### Fixed

- Every matching passkey now reaches the browser's account picker.  When
  the phone answered a sign-in with more than one credential, only the
  first was relayed and the rest were lost; the follow-up requests now
  continue on the same caBLE session, which is closed when the account
  picker is abandoned instead of making the next request fail as busy.

- A failed BLE handshake now falls back to the WebSocket tunnel instead
  of failing the ceremony: the advertisement's channel selector is not
  integrity-protected, so a handshake failure there can mean channel
  steering rather than a phone problem.

- A failed phone handshake or an unexpected tunnel message is now
  reported as a failed attempt rather than as "no passkeys found".
  Firefox still shows a generic not-allowed error for both, so this is
  about protocol accuracy rather than a different browser prompt.

- A browser that stops reading a passkey response can no longer stall
  the daemon: each HID report write is bounded, a whole response has a
  deadline, and a response that cannot be completed is abandoned.  The
  host still learns about the truncation from its own timeout, but the
  daemon and the HID channel are free for the next request.

- A stale reply from a cancelled transaction can no longer be delivered
  as the answer to the next request on the same HID channel.

- Several CTAP-HID state-machine details were tightened: a multi-frame
  request that arrives while a transaction is in progress is refused
  instead of overwriting it, truncated multi-frame requests expire after
  10 seconds even if frames keep trickling in, an INIT resynchronization
  closes an idle multi-credential session, and the reserved channel 0,
  stray continuation frames and LOCK are answered per the specification.

- Starting a second daemon now fails with a clear error instead of
  registering a second virtual authenticator alongside the first.

- Registering a new UI agent now aborts the transaction bound to the
  previous one instead of holding the HID channel until its timeout.

### Security

- Each passkey QR code now carries a fresh, per-transaction identity key
  instead of the installation's persistent one, matching Chromium.  QR
  codes shown by one installation can no longer be recognised as coming
  from the same machine — neither by a phone that scans them nor by
  anyone who records several of them.

- QR public keys are now checked to be valid P-256 points before use,
  so a crafted QR code cannot feed an invalid key into the handshake.

- The installer pins `PATH`, starts itself through an absolute `/bin/sh`
  instead of `env`, and rejects a `setfacl`/`nologin` that is not a
  root-owned, non-writable file (checking the symlink target and baking
  that in) before either ends up in a root-executed udev rule or the
  service account's shell.  A poisoned environment can no longer turn
  installation into root code execution.

- Release tag verification compares the signing-key and primary-key
  fingerprints against a repository variable, so an account with push
  access but without the release key can no longer substitute the key
  stored in the tagged tree, and a future signing subkey keeps working.

- The helper contract is v3: the relying party is passed as `--rp
  <value>` and the transaction URL is only read from stdin, so a relying
  party identifier cannot flip helper modes (not even one spelled
  `--help`) and the transaction secret cannot appear in a world-readable
  command line.

- Both UI agents now accept calls only from the current owner of the
  daemon's bus name, and the session agent only trusts name-change
  signals sent by the bus driver, so another local process can no longer
  impersonate the daemon or silence the QR prompt.

- The daemon's state directory (which holds the long-term identity key)
  is created with mode `0700` instead of systemd's `0755` default.

- The production tunnel dial path now refuses cleartext `ws://` URLs,
  including from redirects, so transaction traffic cannot be downgraded.

- Agent registration attempts are limited per user instead of per D-Bus
  connection, so extra connections can no longer multiply the polkit
  authorization work they cause.

- The P-256 implementation moves to `crypto-bigint` 0.7.9, since 0.7.6
  (what 0.2.1 pinned) was yanked upstream.  0.7.9 also fixes truncation
  in `invert_odd_mod` and boundary cases in `mul_mod_special`, both on
  the curve arithmetic the handshake depends on.

## [0.2.1] - 2026-10-24

### Security

- The P-256 handshake now uses `p256` 0.14, which moves the elliptic-curve
  arithmetic to `crypto-bigint` 0.7.6 and removes timing variability in
  operations intended to be constant-time (RUSTSEC-2026-0387).  The UHID
  device also moves to `AsyncFd::register` with `tokio` 1.54.1, so the
  descriptor stays owned for the whole registration (RUSTSEC-2026-0386).

## [0.2.0] - 2026-10-23

### Added

- New optional GNOME Shell extension `contrib/ucable-gnome-agent`
  (NixOS: `services.ucabled.gnomeAgent.enable`): the extension registers
  as the session UI agent itself and shows the passkey QR prompt and the
  device-selection question as shell modal dialogs, replacing the GTK
  helper window on GNOME.  Its dialogs are translated into the same
  languages as the GTK helper.

- GitHub releases now carry the Debian/Ubuntu packages: the three `.deb`
  packages built for Debian 13 and Ubuntu 26.04 are attached to each
  release, each with a detached GPG signature.

- Stable (non-flake) Nix support: the top-level `default.nix` builds
  the packages with plain `nix-build -A ucabled` and exposes the NixOS
  module as `(import ./. { }).nixosModule`, pinning nixpkgs to the same
  `flake.lock` the flake uses, so the Cachix binary cache applies to
  both entry points.  `nix-shell` now uses that pin as well instead of
  the ambient `<nixpkgs>` channel.

### Changed

- The gettext domain is renamed from `ucable-agent-helper-gtk` to
  `ucable`, and the GTK helper and the GNOME Shell extension now share
  the same catalogs in `po/`.

- Faster sign-in round trips: the CTAPHID, hidraw and tunnel paths copy
  less, and the request path parses and allocates less per command.
- Debian/Ubuntu packaging: split the monolithic `ucabled` package into
  `ucabled` (daemon), `ucable-agent` (default GTK4 session agent) and
  `ucable-gnome-agent` (GNOME Shell extension).  Upgrades pull the
  default agent in through a recommendation; the maintainer scripts
  reload the bus, polkit and udev policy and restart the daemon and the
  logged-in session agents.

### Fixed

- Experimental BLE data channel (`l2cap` feature): frame each caBLE
  message with the 4-byte big-endian length prefix the hybrid
  specification requires, instead of relying on L2CAP message
  boundaries.

### Security

- Wipe transaction secrets more thoroughly: the AES ciphers now erase
  their expanded round keys on drop (the `aes` crate's `zeroize`
  feature), the HKDF intermediate buffers in the Noise handshake are
  erased after use, and all hand-written erase calls moved to the
  `zeroize` crate (already in the dependency tree through p256).

## [0.1.6] - 2026-10-21

### Security

- Update the tunnel's certificate verifier to `rustls-webpki` 0.103.16,
  which could previously accept distrusted intermediates
  (RUSTSEC-2026-0382), and the bundled Mozilla root store to
  `webpki-roots` 1.0.10, which carried the distrusted emSign
  intermediates (RUSTSEC-2026-0381).

## [0.1.5] - 2026-10-20

### Added

- Debian/Ubuntu packaging: `debian/` builds a `ucabled` binary package
  with the systemd units, udev rule, D-Bus and polkit policies, manual
  pages, shell completions and translations.

### Changed

- Link against the gettext runtime that glibc and musl already provide
  instead of letting `gettext-sys` compile and statically link its bundled
  GNU gettext.  This shortens compiles and fixes `dpkg-buildpackage`,
  whose `-flto=auto` link flags broke the bundled libintl archive.

### Fixed

- The Firefox and Chromium snaps can open the virtual token again: the udev
  rule now tags the hidraw node for those snaps.  snapd's `u2f-devices`
  interface only tags devices from a hardcoded USB VID/PID allowlist, which
  can never match a uhid device, so a sandboxed browser saw the token on the
  bus but was refused by its device cgroup.

## [0.1.4] - 2026-10-19

### Security

- Update `rustls` to 0.23.46, fixing four advisories in the tunnel's
  TLS stack (RUSTSEC-2026-0346 to -0349).

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

[0.4.0-pre]: https://github.com/fuzy112/ucabled/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/fuzy112/ucabled/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/fuzy112/ucabled/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/fuzy112/ucabled/compare/v0.1.6...v0.2.0
[0.1.6]: https://github.com/fuzy112/ucabled/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/fuzy112/ucabled/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/fuzy112/ucabled/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/fuzy112/ucabled/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/fuzy112/ucabled/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/fuzy112/ucabled/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/fuzy112/ucabled/releases/tag/v0.1.0
