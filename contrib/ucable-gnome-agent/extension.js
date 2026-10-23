// SPDX-License-Identifier: GPL-3.0-or-later
//
// ucabled GNOME Shell agent.
//
// This extension *is* the session UI agent: it connects to the system bus,
// exports an org.ucabled.Agent1 object and registers it with the daemon's
// org.ucabled.Manager1 (see docs/system-service.md). The daemon only cares
// about the caller's unique bus name plus a polkit allow_active check, both
// of which gnome-shell satisfies exactly like the ucable-agent process.
// Prompts are rendered as shell modal dialogs, so they sit above everything
// the compositor draws.
//
// Lifecycle notes:
// - Shell disable/restart drops the system-bus connection; the daemon's
//   NameOwnerChanged handling then aborts any in-flight transaction
//   (AgentGone). On Wayland a shell crash ends the session anyway.
// - A reappearing org.ucabled name (daemon restart) triggers
//   re-registration; RegisterAgent unconditionally replaces the slot.
// - If this extension and the ucable-agent user service are both active,
//   they fight over the single agent slot (last registration wins). Disable
//   one of them; see README.md.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import {Extension, gettext as _} from 'resource:///org/gnome/shell/extensions/extension.js';

import qrcode from './qrcode.mjs';

const BUS_NAME = 'org.ucabled';
const MANAGER_PATH = '/org/ucabled/Manager';
const MANAGER_IFACE = 'org.ucabled.Manager1';
const AGENT_PATH = '/org/ucabled/GnomeAgent';
const AGENT_IFACE = 'org.ucabled.Agent1';
const ERR_NOT_AUTHORIZED = 'org.ucabled.NotAuthorized';

/// Registration is retried on transient failures (daemon not up yet); the
/// daemon rate-limits one attempt per second per sender, so 2s stays clear.
const REGISTER_RETRY_MS = 2000;

/// The daemon waits 70s for a SelectionResult; answer "declined" after 60s,
/// mirroring the reference GTK helper.
const SELECT_TIMEOUT_SECS = 60;

/// QR quiet zone in modules, matching the GTK helper.
const QR_QUIET = 4;

/// SVG raster resolution in pixels per module.  Without an explicit
/// width/height librsvg renders the code at one pixel per module and
/// St.Icon has to scale that tiny bitmap up, which blurs the modules.
/// The GTK helper rasterizes at the same 10 px per module.
const QR_MODULE_PX = 10;

/// Side of the centre passkey glyph in modules, and the smallest QR version
/// used with it: at 37 modules the 10x10 patch stays well inside the 15%
/// error-correction budget, exactly like the GTK helper.
const QR_CENTER_MODULES = 10;
const QR_MIN_MODULES = 37;

/// The helper's 100x100 passkey mask (src/bin/resources/passkey.gray) as an
/// opaque grey PNG, inlined so the extension ships no side files.  Split over
/// several lines only to keep the source readable.
const PASSKEY_PNG = [
    'iVBORw0KGgoAAAANSUhEUgAAAGQAAABkCAAAAABVicqIAAAD3klEQVR42u3ZW0gUURgA4LO6',
    '3mtpUdao2AxfIiUSHzQxH0JFUBQMNSy6PIiQGaVEYQ8KmrlGiYIFEj1kXtAupqFYKZKVtfnQ',
    'DUlNtzQzo8xLumquW+nuzLhzLnPszD7t/3jmP/PN5cyZ/8wAsx0COBAH4kAciANxIFwY3z28',
    '1djxySQfYsgPdQHLoY6/8UsWpD/FGQjCu2CWObKkcwc24f+UMTITD8ThcoUpMhUOoFHAEJnb',
    'CxBRxg5JRxnA6QErpAmgY9MkG2TOD4OAU2yQCpwBPL4yQYKwCChigfTiDRDEArlKQBTfGSDp',
    'BAS0MUAiSUgFAySYhOgYIDtJSD4DJISEXGKAJJCQKgbIaRLykgFym2B4zjNAxt3wSByTuSsR',
    'j9QyQbqwhnaBzUsrDofcXJU61N18Xz9sWgNiUKGN6CU+r+2YdqVRc+SOifodX69AGZvHuKRn',
    'EcINgU3UJdFFhLHhtTVjMdt2W8oMbXFXBD0XHz1XmEHuW/AwbZla6SXeS2Avdx4xsGPwH6Ut',
    'uHv22OxCmcVfjyz41QxboK3ql+7uFvR3P/ye39SCGhfFa1gE9ZbsC1A7ufuGpFVNCJpNu1CI',
    'epzZcq6W3+v2hmljh+DCnmOG8G+cwOUT/B3L33tWiJEfeI8skwu/JuthhDzmR8OiqC4op0Km',
    'uqsvnEyNCg/+F9EHs5sht2S9tSlUfFNIiLFDl6gVjZwc6+ZSvq3Psg7gZ9Q0SUjf5RhP6PBU',
    'TlsyzvNtB1ZaCvmWVDIyUIgp6V9ZksoFbRl/5YViwVo8g4DM10UqcK8r612pEzaqomI10MoP',
    'ikwUbiSUKdcsmR9xSa0Y5OdZFancAiesyVvROa6TaKTBF5AjzJqdg87Zj37idQoJBnCzHuWQ',
    'EpnzBImUAWlRY+2QicpIRk71z5USkQRuNtDCE7xHUMhigEQDOA9wxwV9Xl3aka/f60ByHOc6',
    'NbpCRlYNupAIlY649vFTsUb0VmxFl0QjCukIiOX7DSfb3LAvmOLuHqAJ4cq3M4lfZER0YcvU',
    'UirE642w7weu3c+ARfKpELDls6DvN4BWViG5dAgIGIEiIuW/ELCtH4oAv1GGCPAdhCI2n9uA',
    'lFUCJpKokWp6REN9ucacqBEvGKIdwK204pgg+CFsfutGi/iIEa2BsDCtVFAi0WKkhbjErvWg',
    'MpSdYqSfvI4fPLSO4mLVQx7GTCkfC+b0jXXSon0WOq3kyvbjTDiEz9gDsVXkQWy+UcmEHJUH',
    '+WGPMzELv1rkyfVbVq9CjmGG/3671fDzYPuD2aLkmeVEVpQ8s7yI+cUOdYlZbmTt/34diJ2R',
    'P1B9arNDA72uAAAAAElFTkSuQmCC',
].join('');

const AGENT_XML = `
<node>
  <interface name="org.ucabled.Agent1">
    <method name="Prompt">
      <arg type="t" direction="in" name="tid"/>
      <arg type="s" direction="in" name="url"/>
      <arg type="s" direction="in" name="rp"/>
      <arg type="t" direction="in" name="timeout_secs"/>
      <arg type="s" direction="in" name="request_type"/>
    </method>
    <method name="Select">
      <arg type="t" direction="in" name="tid"/>
    </method>
    <method name="Found">
      <arg type="t" direction="in" name="tid"/>
    </method>
    <method name="Close">
      <arg type="t" direction="in" name="tid"/>
    </method>
    <method name="Notify">
      <arg type="s" direction="in" name="summary"/>
      <arg type="s" direction="in" name="body"/>
    </method>
  </interface>
</node>`;

/// The caption under the QR code, phrased after the CTAP command waiting on
/// the other end.  Translations are shared with the GTK helper through the
/// "ucable" gettext domain (po/).
function scanCaption(requestType) {
    switch (requestType) {
    case 'mc':
        return _('Scan with your phone\nto store a passkey');
    case 'ga':
        return _('Scan with your phone\nto authenticate with passkeys');
    default:
        return _('Scan with your phone');
    }
}

/// Is (row, col) inside one of the three 7x7 finder patterns?
function isFinder(row, col, n) {
    const top = row < 7;
    const bottom = row >= n - 7;
    const left = col < 7;
    const right = col >= n - 7;
    return (top && (left || right)) || (bottom && left);
}

/// One finder pattern as three nested rounded rectangles, in module units.
function finderSvg(x, y) {
    return `<rect x="${x}" y="${y}" width="7" height="7" rx="1.2" fill="#000"/>` +
        `<rect x="${x + 1}" y="${y + 1}" width="5" height="5" rx="0.8" fill="#fff"/>` +
        `<rect x="${x + 2}" y="${y + 2}" width="3" height="3" rx="0.6" fill="#000"/>`;
}

/// Rasterize a caBLE URL into an in-memory SVG: round data modules, rounded
/// finder patterns and the passkey glyph in the middle, like the GTK helper.
/// The URL is secret and must never be logged or persisted — the SVG lives
/// only in this process's memory (GBytesIcon, never written to disk).
function qrSvgBytes(url) {
    let qr = qrcode(0, 'M');
    qr.addData(url);
    qr.make();

    // The centre glyph covers data modules; bump tiny payloads to version 5
    // (37 modules) so error correction still has enough slack.
    if (qr.getModuleCount() < QR_MIN_MODULES) {
        qr = qrcode(5, 'M');
        qr.addData(url);
        qr.make();
    }

    const n = qr.getModuleCount();
    const size = n + QR_QUIET * 2;

    const circles = [];
    for (let row = 0; row < n; row++) {
        for (let col = 0; col < n; col++) {
            if (isFinder(row, col, n) || !qr.isDark(row, col))
                continue;
            circles.push(
                `<circle cx="${col + QR_QUIET + 0.5}" cy="${row + QR_QUIET + 0.5}" r="0.42"/>`);
        }
    }

    const far = QR_QUIET + n - 7;
    const center = Math.floor((n - QR_CENTER_MODULES) / 2) + QR_QUIET;
    const svg = `<svg xmlns="http://www.w3.org/2000/svg"` +
        ` width="${size * QR_MODULE_PX}" height="${size * QR_MODULE_PX}"` +
        ` viewBox="0 0 ${size} ${size}">` +
        `<rect width="${size}" height="${size}" fill="#fff"/>` +
        `<g fill="#000">${circles.join('')}</g>` +
        finderSvg(QR_QUIET, QR_QUIET) +
        finderSvg(far, QR_QUIET) +
        finderSvg(QR_QUIET, far) +
        `<rect x="${center}" y="${center}"` +
        ` width="${QR_CENTER_MODULES}" height="${QR_CENTER_MODULES}" fill="#fff"/>` +
        `<image x="${center}" y="${center}"` +
        ` width="${QR_CENTER_MODULES}" height="${QR_CENTER_MODULES}"` +
        ` href="data:image/png;base64,${PASSKEY_PNG}"/>` +
        '</svg>';
    return new GLib.Bytes(new TextEncoder().encode(svg));
}

/// QR prompt dialog. Closing it with Cancel/Esc reports
/// TransactionCancelled; daemon-initiated Close() goes through
/// closeFromDaemon(), which suppresses that report (mirroring the helper
/// contract, where a SIGKILLed helper reports nothing).
const QrDialog = GObject.registerClass(
class QrDialog extends ModalDialog.ModalDialog {
    _init({url, rp, requestType, onUserClose}) {
        super._init({styleClass: 'prompt-dialog', destroyOnClose: true});
        this._onUserClose = onUserClose;
        this._closedByDaemon = false;

        const layout = new St.BoxLayout({vertical: true});
        layout.set_style('spacing: 12px;');

        if (rp) {
            layout.add_child(new St.Label({
                text: rp,
                style_class: 'prompt-dialog-headline',
                x_align: Clutter.ActorAlign.CENTER,
            }));
        }

        this._qrIcon = new St.Icon({
            gicon: Gio.BytesIcon.new(qrSvgBytes(url)),
            icon_size: 260,
            x_align: Clutter.ActorAlign.CENTER,
        });
        layout.add_child(this._qrIcon);

        this._caption = new St.Label({
            text: scanCaption(requestType),
            style_class: 'prompt-dialog-description',
            x_align: Clutter.ActorAlign.CENTER,
        });
        layout.add_child(this._caption);

        this.contentLayout.add_child(layout);

        this.setButtons([{
            label: _('Cancel'),
            action: () => this.close(global.get_current_time()),
            key: Clutter.KEY_Escape,
            default: true,
        }]);

        this.connect('closed', () => {
            if (!this._closedByDaemon)
                this._onUserClose?.();
        });
    }

    /// The daemon saw the phone's BLE advert: dim the code (St has no blur
    /// filter, unlike the GTK helper) and ask to confirm on the phone.
    markFound() {
        // markExpired() destroys the icon; a late Found must not touch it.
        if (this._qrIcon)
            this._qrIcon.opacity = 110;
        this._caption.text =
            _('Phone detected.\nConfirm the sign-in on your phone.');
    }

    /// Like the reference helper, an expired code is announced but the
    /// dialog stays until the user closes it.
    markExpired() {
        this._qrIcon.destroy();
        this._qrIcon = null;
        this._caption.text =
            _('Code expired.\nClose this dialog and start again.');
    }

    closeFromDaemon() {
        this._closedByDaemon = true;
        this.close(global.get_current_time());
    }
});

/// Device-selection dialog: answer flows back through SelectionResult.
/// Closing without a button (Escape maps to the decline button, so this is
/// only a fallback) counts as "declined".
const SelectDialog = GObject.registerClass(
class SelectDialog extends ModalDialog.ModalDialog {
    _init({onAnswer}) {
        super._init({styleClass: 'prompt-dialog', destroyOnClose: true});
        this._answered = false;
        this._onAnswer = onAnswer;

        const layout = new St.BoxLayout({vertical: true});
        layout.set_style('spacing: 12px;');
        layout.add_child(new St.Label({
            text: _('Passkey request'),
            style_class: 'prompt-dialog-headline',
            x_align: Clutter.ActorAlign.CENTER,
        }));
        layout.add_child(new St.Label({
            text: _('A security key is also connected.\n' +
                'Use your phone’s passkey instead?'),
            style_class: 'prompt-dialog-description',
            x_align: Clutter.ActorAlign.CENTER,
        }));
        this.contentLayout.add_child(layout);

        this.setButtons([
            {
                label: _('Use security key'),
                action: () => this.answer(false),
                key: Clutter.KEY_Escape,
                default: true,
            },
            {
                label: _('Use phone'),
                action: () => this.answer(true),
            },
        ]);

        this.connect('closed', () => {
            if (!this._answered)
                this.answer(false);
        });
    }

    answer(usePhone) {
        if (this._answered)
            return;
        this._answered = true;
        this._onAnswer?.(usePhone);
        this.close(global.get_current_time());
    }

    closeFromDaemon() {
        this._answered = true;
        this.close(global.get_current_time());
    }
});

export default class UcabledAgentExtension extends Extension {
    enable() {
        // Bind the "ucable" domain (shared with the GTK helper) to this
        // extension's locale/ directory; every _() lookup happens at
        // runtime, after this point.
        this.initTranslations();

        this._dialogs = new Map(); // tid -> QrDialog | SelectDialog
        this._timers = new Set(); // GLib source ids
        this._registerTimer = null;
        this._registered = false;

        // A plain facade object, so wrapJSObject does not have to walk the
        // extension class prototype.
        this._agentFacade = {
            Prompt: (tid, url, rp, timeoutSecs, requestType) =>
                this._prompt(tid, url, rp, timeoutSecs, requestType),
            Select: tid => this._select(tid),
            Found: tid => this._found(tid),
            Close: tid => this._close(tid),
            Notify: (summary, body) => Main.notify(summary, body),
        };
        this._dbusObject = Gio.DBusExportedObject.wrapJSObject(
            AGENT_XML, this._agentFacade);
        this._dbusObject.export(Gio.DBus.system, AGENT_PATH);

        // Defence in depth: the shipped bus policy already denies
        // unprivileged sends to our unique name, so only the daemon can reach
        // this object.  Filtering the method calls by sender as well keeps a
        // missing or relaxed org.ucabled.conf from handing the daemon's
        // authority to another local process.
        this._daemonOwner = null;
        this._agentFilter = null;
        if (typeof Gio.DBus.system.add_filter === 'function') {
            this._agentFilter = Gio.DBus.system.add_filter(
                (_conn, message, incoming) => {
                    if (!incoming ||
                        message.get_message_type() !==
                            Gio.DBusMessageType.METHOD_CALL ||
                        message.get_path() !== AGENT_PATH)
                        return message;
                    // Fail closed: the filter is the only check when the bus
                    // policy is missing or relaxed, so a call is accepted
                    // only when it provably comes from the current daemon.
                    // Until watch_name has resolved the owner (and after the
                    // name disappears) every call is refused.
                    const sender = message.get_sender();
                    if (!this._daemonOwner ||
                        message.get_interface() !== AGENT_IFACE ||
                        sender !== this._daemonOwner) {
                        console.warn(
                            `ucabled: ignoring agent call from ${sender}`);
                        return null;
                    }
                    return message;
                });
        } else {
            console.warn(
                'ucabled: Gio.DBus.system.add_filter is unavailable; ' +
                'only the shipped bus policy guards the agent object');
        }

        // Register now, and again whenever the daemon's name reappears.
        this._nameWatch = Gio.DBus.system.watch_name(
            BUS_NAME, Gio.BusNameWatcherFlags.NONE,
            (_conn, _name, owner) => {
                this._daemonOwner = owner;
                this._register();
            },
            () => {
                this._daemonOwner = null;
                this._registered = false;
            });
        this._register();
    }

    disable() {
        if (this._registered)
            this._callManager('UnregisterAgent', null);
        this._registered = false;

        if (this._registerTimer) {
            GLib.source_remove(this._registerTimer);
            this._registerTimer = null;
        }
        if (this._nameWatch) {
            Gio.DBus.system.unwatch_name(this._nameWatch);
            this._nameWatch = null;
        }
        if (this._agentFilter && typeof Gio.DBus.system.remove_filter === 'function') {
            Gio.DBus.system.remove_filter(this._agentFilter);
            this._agentFilter = null;
        }
        for (const id of this._timers)
            GLib.source_remove(id);
        this._timers.clear();

        for (const [, dialog] of this._dialogs)
            dialog.closeFromDaemon();
        this._dialogs.clear();

        this._dbusObject?.unexport();
        this._dbusObject = null;
    }

    _register() {
        Gio.DBus.system.call(
            BUS_NAME, MANAGER_PATH, MANAGER_IFACE, 'RegisterAgent',
            new GLib.Variant('(o)', [AGENT_PATH]),
            null, Gio.DBusCallFlags.NONE, -1, null,
            (_conn, res) => {
                try {
                    Gio.DBus.system.call_finish(res);
                    this._registered = true;
                    console.log('ucabled: registered as UI agent');
                } catch (e) {
                    let remote = null;
                    try {
                        remote = Gio.DBusError.get_remote_error(e);
                    } catch (_) {
                        // Not a remote D-Bus error at all.
                    }
                    if (remote === ERR_NOT_AUTHORIZED) {
                        // Final: only the active local session may register.
                        console.error(
                            `ucabled: agent registration denied by polkit; not retrying: ${e.message}`);
                        return;
                    }
                    console.warn(
                        `ucabled: agent registration failed, retrying: ${e.message}`);
                    if (this._registerTimer)
                        return;
                    this._registerTimer = GLib.timeout_add(
                        GLib.PRIORITY_DEFAULT, REGISTER_RETRY_MS, () => {
                            this._registerTimer = null;
                            this._register();
                            return GLib.SOURCE_REMOVE;
                        });
                }
            });
    }

    /// Fire-and-forget call on the daemon's Manager1 (TransactionCancelled,
    /// SelectionResult, UnregisterAgent); failures are only logged.
    _callManager(method, params) {
        Gio.DBus.system.call(
            BUS_NAME, MANAGER_PATH, MANAGER_IFACE, method, params,
            null, Gio.DBusCallFlags.NONE, -1, null,
            (_conn, res) => {
                try {
                    Gio.DBus.system.call_finish(res);
                } catch (e) {
                    console.warn(`ucabled: ${method} failed: ${e.message}`);
                }
            });
    }

    /// Defensively drop whatever is on screen: the daemon closes stale
    /// dialogs before prompting, but a replacement prompt must never stack.
    _closeAll() {
        for (const [, dialog] of this._dialogs)
            dialog.closeFromDaemon();
        this._dialogs.clear();
    }

    _cancelTransaction(tid) {
        this._dialogs.delete(tid);
        this._callManager('TransactionCancelled',
            new GLib.Variant('(t)', [tid]));
    }

    /// Open a dialog; on failure (e.g. locked screen cannot grab the modal)
    /// run `onFailure` so the daemon is not left hanging.
    _open(tid, dialog, onFailure) {
        this._dialogs.set(tid, dialog);
        if (!dialog.open(global.get_current_time())) {
            console.warn('ucabled: cannot grab the modal focus (locked screen?); prompt not shown');
            this._dialogs.delete(tid);
            dialog.destroy();
            onFailure();
        }
    }

    _prompt(tid, url, rp, timeoutSecs, requestType) {
        this._closeAll();

        let dialog;
        try {
            dialog = new QrDialog({
                url,
                rp,
                requestType,
                onUserClose: () => this._cancelTransaction(tid),
            });
        } catch (e) {
            console.error(`ucabled: failed to build QR dialog: ${e.message}`);
            this._cancelTransaction(tid);
            return;
        }

        this._open(tid, dialog, () => this._cancelTransaction(tid));
        if (!this._dialogs.has(tid))
            return;

        if (timeoutSecs > 0) {
            const id = GLib.timeout_add_seconds(
                GLib.PRIORITY_DEFAULT, timeoutSecs, () => {
                    this._timers.delete(id);
                    if (this._dialogs.get(tid) === dialog)
                        dialog.markExpired();
                    return GLib.SOURCE_REMOVE;
                });
            this._timers.add(id);
        }
    }

    _select(tid) {
        this._closeAll();

        const dialog = new SelectDialog({
            onAnswer: usePhone => {
                this._dialogs.delete(tid);
                this._callManager('SelectionResult',
                    new GLib.Variant('(tb)', [tid, usePhone]));
            },
        });

        // If the dialog cannot open at all, decline immediately rather than
        // let the daemon wait out its 70s selection deadline.
        this._open(tid, dialog, () =>
            this._callManager('SelectionResult',
                new GLib.Variant('(tb)', [tid, false])));
        if (!this._dialogs.has(tid))
            return;

        const id = GLib.timeout_add_seconds(
            GLib.PRIORITY_DEFAULT, SELECT_TIMEOUT_SECS, () => {
                this._timers.delete(id);
                if (this._dialogs.get(tid) === dialog)
                    dialog.answer(false);
                return GLib.SOURCE_REMOVE;
            });
        this._timers.add(id);
    }

    _found(tid) {
        const dialog = this._dialogs.get(tid);
        if (dialog instanceof QrDialog)
            dialog.markFound();
    }

    _close(tid) {
        const dialog = this._dialogs.get(tid);
        if (!dialog)
            return;
        this._dialogs.delete(tid);
        dialog.closeFromDaemon();
    }
}
