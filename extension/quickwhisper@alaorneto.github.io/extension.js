// QuickWhisper GNOME Shell extension: listens to the daemon's D-Bus signals,
// drives the recording pill overlay, and auto-pastes the transcription into
// the focused text field with a synthetic Ctrl+V (Clutter virtual keyboard —
// this runs inside the compositor, so it works on Wayland).

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {Overlay} from './overlay.js';

const BUS_NAME = 'io.github.alaor.QuickWhisper';
const OBJECT_PATH = '/io/github/alaor/QuickWhisper';
// Owning this name tells the daemon that overlay + auto-paste are available.
const OVERLAY_BUS_NAME = 'io.github.alaor.QuickWhisper.Overlay';

// Delay between Finished and Ctrl+V: lets the fresh clipboard selection
// propagate and any lingering key state settle.
const PASTE_DELAY_MS = 120;

export default class QuickWhisperExtension extends Extension {
    enable() {
        this._overlay = new Overlay();
        this._pasteTimeout = 0;

        const seat = Clutter.get_default_backend().get_default_seat();
        this._keyboard = seat.create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);

        this._nameId = Gio.DBus.session.own_name(
            OVERLAY_BUS_NAME, Gio.BusNameOwnerFlags.NONE, null, null);

        this._signalId = Gio.DBus.session.signal_subscribe(
            null, BUS_NAME, null, OBJECT_PATH, null,
            Gio.DBusSignalFlags.NONE, (...args) => this._onSignal(...args));

        // If the daemon crashes mid-dictation no terminal signal ever comes;
        // dismiss the pill as soon as its bus name vanishes.
        this._watchId = Gio.bus_watch_name(
            Gio.BusType.SESSION, BUS_NAME, Gio.BusNameWatcherFlags.NONE,
            null, () => this._overlay?.hideNow());
    }

    disable() {
        if (this._watchId) {
            Gio.bus_unwatch_name(this._watchId);
            this._watchId = 0;
        }
        if (this._signalId) {
            Gio.DBus.session.signal_unsubscribe(this._signalId);
            this._signalId = 0;
        }
        if (this._nameId) {
            Gio.DBus.session.unown_name(this._nameId);
            this._nameId = 0;
        }
        if (this._pasteTimeout) {
            GLib.source_remove(this._pasteTimeout);
            this._pasteTimeout = 0;
        }
        this._keyboard = null;
        this._overlay?.destroy();
        this._overlay = null;
    }

    _onSignal(_conn, _sender, _path, _iface, signal, params) {
        switch (signal) {
        case 'RecordingStarted':
            this._overlay.showRecording();
            break;
        case 'AudioLevel':
            this._overlay.setLevel(params.get_child_value(0).get_double());
            break;
        case 'ProcessingStarted':
            this._overlay.showProcessing();
            break;
        case 'Finished':
            this._schedulePaste();
            this._overlay.finishIfProcessing();
            break;
        case 'Failed':
        case 'Cancelled':
            // These can terminate a live recording (quick tap, D-Bus cancel),
            // when the overlay is still in 'recording' — dismiss from any state.
            this._overlay.dismiss();
            break;
        }
    }

    _schedulePaste() {
        if (this._pasteTimeout)
            GLib.source_remove(this._pasteTimeout);
        this._pasteTimeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, PASTE_DELAY_MS, () => {
            this._pasteTimeout = 0;
            this._sendCtrlV();
            return GLib.SOURCE_REMOVE;
        });
    }

    _sendCtrlV() {
        const t = Clutter.get_current_event_time();
        this._keyboard.notify_keyval(t, Clutter.KEY_Control_L, Clutter.KeyState.PRESSED);
        this._keyboard.notify_keyval(t, Clutter.KEY_v, Clutter.KeyState.PRESSED);
        this._keyboard.notify_keyval(t, Clutter.KEY_v, Clutter.KeyState.RELEASED);
        this._keyboard.notify_keyval(t, Clutter.KEY_Control_L, Clutter.KeyState.RELEASED);
    }
}
