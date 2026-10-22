# ucabled GNOME Shell agent

A GNOME Shell extension that **is** the ucabled session UI agent: it
registers an `org.ucabled.Agent1` object with the system daemon (see
`docs/system-service.md`) and renders passkey prompts as shell modal
dialogs, above everything the compositor draws. No helper process, no GTK
window, no temporary files — the QR code is rendered in-memory as an SVG
inside the shell process.

```
daemon ──(system bus, org.ucabled.Agent1)──▶ gnome-shell (this extension)
```

The daemon needs no changes: it only cares about the caller's unique bus
name and a polkit `allow_active` check, both of which gnome-shell
satisfies exactly like the `ucable-agent` process.

## What you get

- **QR prompt** as a shell modal dialog: relying party, QR code, caption
  phrased after the CTAP command (`mc`/`ga`); *Cancel* or *Esc* cancels
  the transaction. When the phone is detected the code dims and the
  caption asks to confirm on the phone. On timeout the dialog announces
  "Code expired" and waits for dismissal, like the reference helper.
- **Device selection** ("Use security key" vs. "Use phone") as a shell
  dialog when a physical security key is also connected.
- **Daemon notices** (e.g. refused tunnel redirects) as shell
  notifications.

## Requirements

- GNOME Shell 45–50 (ESM extensions).
- The `ucabled` system daemon running, with its D-Bus policy and polkit
  action installed (both are part of the normal install).

## Install

### NixOS

```nix
services.ucabled.enable = true;
services.ucabled.gnomeAgent.enable = true;   # installs the extension system-wide
```

then, per user, enable it and re-login (Wayland cannot reload the shell):

```console
$ gnome-extensions enable ucable-gnome-agent@fuzy.me
```

### Manual

```console
$ mkdir -p ~/.local/share/gnome-shell/extensions/ucable-gnome-agent@fuzy.me
$ cp metadata.json extension.js qrcode.mjs \
    ~/.local/share/gnome-shell/extensions/ucable-gnome-agent@fuzy.me/
$ gnome-extensions enable ucable-gnome-agent@fuzy.me
```

Log out and back in (Wayland) or restart the shell (X11: Alt+F2, `r`).

## Coexistence with `ucable-agent` — pick one

The daemon has a **single** agent slot; `RegisterAgent` unconditionally
replaces the current holder. If the `ucable-agent` user service is also
running, the two fight over the slot (last registration wins, and the
loser does not re-register). On a GNOME-only machine, disable the service:

```console
$ systemctl --user disable --now ucable-agent.service
```

On NixOS:

```nix
systemd.user.services.ucable-agent.wantedBy = lib.mkForce [ ];
```

Keep the user service (and no extension) if you also log into non-GNOME
sessions — the extension offers no fallback there.

## Security notes

- The QR URL carries the transaction secret. It reaches this extension
  through the same system-bus **unicast** `Prompt` call that delivers it
  to `ucable-agent` — but the receiving process is now gnome-shell, the
  largest, most extension-loaded process in the session. The QR is
  rendered to an in-memory SVG only; nothing is written to disk or logged.
- polkit authorization is unchanged: only the active local session may
  register, and the daemon re-checks on every `Prompt`.
- If the shell is restarted or the extension disabled mid-transaction,
  the bus connection drops and the daemon aborts the transaction
  (`AgentGone`).

## Limitations

- English-only strings; the translated UI is the GTK helper (`po/`).
- The "phone detected" state dims the QR instead of blurring it (St has
  no blur filter).
- On the lock screen no modal can grab the focus; the prompt is skipped
  and the transaction simply times out (same as the reference design).
- Each GNOME Shell release may break extension APIs; the version list in
  `metadata.json` is what has been considered, not a guarantee.

## Files

- `extension.js` — the agent: D-Bus `Agent1` implementation, registration
  with retry, dialog lifecycle.
- `qrcode.mjs` — vendored MIT QR encoder
  ([kazuhikoarase/qrcode-generator](https://github.com/kazuhikoarase/qrcode-generator),
  `js/dist/qrcode.mjs`). Do not edit; update by re-downloading upstream.
- `metadata.json` — extension manifest.
