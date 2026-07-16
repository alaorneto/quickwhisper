// QuickWhisper recording pill: bottom-center OSD with a voice-reactive
// waveform (purple→orange) while recording and a traveling-wave shimmer
// while transcribing. Rendered as shell chrome, same technique as the
// native volume OSD.
//
// Sizing contract: every dimension and position is computed here from the
// constants below (logical px × UI scale factor) with plain fixed-layout
// actors. Nothing is measured through the CSS box model — a measure-and-lock
// approach proved unreliable because the result depended on *when* the
// stylesheet was resolved, letting the wave outgrow the pill. The stylesheet
// now only paints (colors, border, radius, shadow) and can never affect
// geometry; the wave box also clips to its allocation as a hard guarantee.

import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

const N_BARS = 24;
const BAR_WIDTH = 4;
const BAR_SPACING = 3;
const BAR_MIN = 4;
const BAR_MAX = 26;
const ICON_SIZE = 16;
const GAP = 14;   // between icon and wave
const PAD_X = 21; // pill edge to content, 1px border included
const PILL_H = 48;
const WAVE_W = N_BARS * BAR_WIDTH + (N_BARS - 1) * BAR_SPACING;
const PILL_W = 2 * PAD_X + ICON_SIZE + GAP + WAVE_W;
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
        this._scale = 1;

        // Plain St.Widgets (fixed layout): children sit exactly where
        // _layout() puts them, immune to style-resolution timing.
        this._pill = new St.Widget({
            style_class: 'qw-pill',
            reactive: false,
            visible: false,
        });
        this._icon = new St.Icon({
            icon_name: 'audio-input-microphone-symbolic',
            style_class: 'qw-icon',
            icon_size: ICON_SIZE, // logical px; St.Icon applies the UI scale
        });
        this._pill.add_child(this._icon);

        // Spinner shares the mic icon's slot; shown only while processing.
        this._spinner = new St.Icon({
            icon_name: 'process-working-symbolic',
            style_class: 'qw-icon',
            icon_size: ICON_SIZE,
            opacity: 0,
        });
        this._spinner.set_pivot_point(0.5, 0.5);
        this._pill.add_child(this._spinner);

        this._waveBox = new St.Widget({clip_to_allocation: true});
        this._bars = [];
        for (let i = 0; i < N_BARS; i++) {
            const bar = new St.Widget({style_class: 'qw-bar'});
            bar.set_style(`background-color: ${barColor(i)};`);
            this._bars.push(bar);
            this._waveBox.add_child(bar);
        }
        this._pill.add_child(this._waveBox);

        this._layout();
        St.ThemeContext.get_for_stage(global.stage).connectObject(
            'notify::scale-factor', () => {
                this._layout();
                if (this._state !== 'hidden')
                    this._place();
            }, this._pill);

        // Top chrome so the non-reactive pill floats above windows.
        Main.layoutManager.addTopChrome(this._pill);
    }

    destroy() {
        this._stopTick();
        this._pill.destroy(); // also drops the scale-factor handler
        this._pill = null;
    }

    // (Re)applies the full fixed geometry at the current UI scale.
    _layout() {
        const s = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        this._scale = s;
        this._pill.set_size(PILL_W * s, PILL_H * s);
        this._icon.set_position(PAD_X * s, ((PILL_H - ICON_SIZE) / 2) * s);
        this._spinner.set_position(PAD_X * s, ((PILL_H - ICON_SIZE) / 2) * s);
        this._waveBox.set_position(
            (PAD_X + ICON_SIZE + GAP) * s, ((PILL_H - BAR_MAX) / 2) * s);
        this._waveBox.set_size(WAVE_W * s, BAR_MAX * s);
        this._bars.forEach((bar, i) => {
            bar.set_x(i * (BAR_WIDTH + BAR_SPACING) * s);
            this._setBarHeight(bar, BAR_MIN);
        });
    }

    // Bars are vertically centered in the wave box; heights are in logical
    // px and clamped so a bar can never exceed the box (the box clips too).
    _setBarHeight(bar, h) {
        const s = this._scale;
        const hp = Math.round(Math.min(Math.max(h, 1), BAR_MAX) * s);
        bar.set_size(BAR_WIDTH * s, hp);
        bar.set_y(Math.round((BAR_MAX * s - hp) / 2));
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
        const s = this._scale;
        this._pill.set_position(
            monitor.x + Math.floor((monitor.width - PILL_W * s) / 2),
            monitor.y + monitor.height - (PILL_H + MARGIN_BOTTOM) * s);
    }

    showRecording() {
        this._state = 'recording';
        this._level = 0;
        this._target = 0;
        this._icon.opacity = 255;
        this._spinner.opacity = 0;
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
                this._setBarHeight(b, (BAR_MIN + BAR_MAX) / 2);
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
        // Swap the mic icon for a spinner in the same slot: the pill geometry
        // never changes, so the dots keep their exact place from the
        // recording state. The spinner rotates from _onTick().
        this._icon.opacity = 0;
        this._spinner.rotation_angle_z = 0;
        this._spinner.opacity = 255;
        if (!this._animationsEnabled()) {
            // Reduced motion: static spinner glyph, dimmed dots.
            this._bars.forEach(b => {
                this._setBarHeight(b, BAR_MIN);
                b.opacity = 160;
            });
        }
    }

    // A dictation Finished. Only dismiss when we're not already showing a
    // newer, chained recording.
    finishIfProcessing() {
        if (this._state !== 'processing')
            return;
        this.dismiss();
    }

    // Fade the pill out from any state — used for Failed/Cancelled, which can
    // arrive while still 'recording' (quick tap discarded by the daemon).
    dismiss() {
        if (this._state === 'hidden')
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

    // Immediate dismissal, no fade — used when the daemon vanishes from the
    // bus mid-dictation and no terminal signal will ever arrive.
    hideNow() {
        this._state = 'hidden';
        this._stopTick();
        this._pill.remove_all_transitions();
        this._pill.hide();
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
                this._setBarHeight(this._bars[i], h);
                this._bars[i].opacity = 255;
            }
        } else if (this._state === 'processing') {
            // Bars collapse to a line; a brightness wave travels through the
            // gradient while whisper works, and the spinner turns in the
            // mic icon's slot.
            this._spinner.rotation_angle_z = (this._phase * 240) % 360;
            for (let i = 0; i < N_BARS; i++) {
                this._setBarHeight(this._bars[i], BAR_MIN);
                const wave = 0.5 + 0.5 * Math.sin(this._phase * 4 - i * 0.45);
                this._bars[i].opacity = Math.round(90 + 165 * wave);
            }
        }
    }
}
