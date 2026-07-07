// QuickWhisper recording pill: bottom-center OSD with a voice-reactive
// waveform (purple→orange) while recording and a traveling-wave shimmer
// while transcribing. Rendered as shell chrome, same technique as the
// native volume OSD.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const N_BARS = 24;
const BAR_WIDTH = 4;
const BAR_MIN = 4;
const BAR_MAX = 26;
const TICK_MS = 33; // ~30 fps
const MARGIN_BOTTOM = 64;

const PURPLE = [0x7c, 0x3a, 0xed];
const ORANGE = [0xf9, 0x73, 0x16];

function barColor(i) {
    const t = i / (N_BARS - 1);
    const c = PURPLE.map((p, k) => Math.round(p + (ORANGE[k] - p) * t));
    return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

export class Overlay {
    constructor() {
        this._state = 'hidden'; // hidden | recording | processing
        this._level = 0;        // smoothed mic level 0..1
        this._target = 0;
        this._phase = 0;
        this._tick = 0;

        this._pill = new St.BoxLayout({
            style_class: 'qw-pill',
            reactive: false,
            visible: false,
        });
        this._icon = new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'qw-icon',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._pill.add_child(this._icon);

        this._barsBox = new St.BoxLayout({
            style_class: 'qw-bars',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._bars = [];
        for (let i = 0; i < N_BARS; i++) {
            const bar = new St.Widget({
                style_class: 'qw-bar',
                y_align: Clutter.ActorAlign.CENTER,
            });
            bar.set_style(`background-color: ${barColor(i)};`);
            bar.set_size(BAR_WIDTH, BAR_MIN);
            this._bars.push(bar);
            this._barsBox.add_child(bar);
        }
        this._pill.add_child(this._barsBox);

        // Top chrome so the pill floats above windows; it must never eat input.
        Main.layoutManager.addTopChrome(this._pill, {affectsInputRegion: false});
    }

    destroy() {
        this._stopTick();
        this._pill.destroy();
        this._pill = null;
    }

    _animationsEnabled() {
        return St.Settings.get().enable_animations;
    }

    // Bottom-center of the monitor the pointer is on (dictation follows focus,
    // and focus usually follows the pointer).
    _place() {
        const [px, py] = global.get_pointer();
        const monitor =
            Main.layoutManager.monitors.find(
                m => px >= m.x && px < m.x + m.width && py >= m.y && py < m.y + m.height) ??
            Main.layoutManager.primaryMonitor;
        const [, natW] = this._pill.get_preferred_width(-1);
        const [, natH] = this._pill.get_preferred_height(-1);
        this._pill.set_position(
            monitor.x + Math.floor((monitor.width - natW) / 2),
            monitor.y + monitor.height - natH - MARGIN_BOTTOM);
    }

    showRecording() {
        this._state = 'recording';
        this._level = 0;
        this._target = 0;
        this._pill.remove_all_transitions();
        this._pill.show();
        this._place();

        if (this._animationsEnabled()) {
            this._pill.opacity = 0;
            this._pill.ease({
                opacity: 255,
                duration: 150,
                mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            });
            this._startTick();
        } else {
            // Reduced motion: static pill, mid-height bars, no waveform.
            this._pill.opacity = 255;
            this._bars.forEach(b => {
                b.height = (BAR_MIN + BAR_MAX) / 2;
                b.opacity = 255;
            });
        }
    }

    setLevel(level) {
        this._target = Math.min(Math.max(level, 0), 1);
    }

    showProcessing() {
        if (this._state === 'hidden')
            return;
        this._state = 'processing';
        if (!this._animationsEnabled()) {
            this._bars.forEach(b => {
                b.height = BAR_MIN;
                b.opacity = 160;
            });
        }
    }

    // A dictation reached a terminal state (Finished/Failed/Cancelled). Only
    // dismiss when we're not already showing a newer, chained recording.
    finishIfProcessing() {
        if (this._state !== 'processing')
            return;
        this._state = 'hidden';
        if (this._animationsEnabled()) {
            this._pill.remove_all_transitions();
            this._pill.ease({
                opacity: 0,
                duration: 200,
                mode: Clutter.AnimationMode.EASE_IN_QUAD,
                onComplete: () => {
                    this._pill.hide();
                    this._stopTick();
                },
            });
        } else {
            this._pill.hide();
        }
    }

    _startTick() {
        if (this._tick)
            return;
        this._tick = GLib.timeout_add(GLib.PRIORITY_DEFAULT, TICK_MS, () => {
            this._onTick();
            return GLib.SOURCE_CONTINUE;
        });
    }

    _stopTick() {
        if (this._tick) {
            GLib.source_remove(this._tick);
            this._tick = 0;
        }
    }

    _onTick() {
        this._phase += TICK_MS / 1000;
        if (this._state === 'recording') {
            // Ease toward the daemon-reported level (~15 Hz) for fluid motion.
            this._level += (this._target - this._level) * 0.35;
            for (let i = 0; i < N_BARS; i++) {
                const sway = 0.35 + 0.65 * Math.abs(Math.sin(this._phase * 2.4 + i * 0.55));
                const h = BAR_MIN + (BAR_MAX - BAR_MIN) * this._level * sway;
                this._bars[i].height = Math.round(h);
                this._bars[i].opacity = 255;
            }
        } else if (this._state === 'processing') {
            // Bars collapse to a line; a brightness wave travels through the
            // gradient while whisper works.
            for (let i = 0; i < N_BARS; i++) {
                this._bars[i].height = BAR_MIN;
                const wave = 0.5 + 0.5 * Math.sin(this._phase * 4 - i * 0.45);
                this._bars[i].opacity = Math.round(90 + 165 * wave);
            }
        }
    }
}
