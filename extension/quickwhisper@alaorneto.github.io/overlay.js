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
const MONITOR_MODES = new Set(['pointer', 'primary', 'all']);

function barColor(i) {
    const t = i / (N_BARS - 1);
    const c = PURPLE.map((p, k) => Math.round(p + (ORANGE[k] - p) * t));
    return `rgb(${c[0]}, ${c[1]}, ${c[2]})`;
}

// One visual tree bound to one current monitor. It owns no logical state and
// no timer; the Overlay controller drives every view from the same frame.
class OverlayView {
    constructor(monitor) {
        this._monitor = monitor;
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

        this.layout();

        // Top chrome so the non-reactive pill floats above windows.
        Main.layoutManager.addTopChrome(this._pill);
    }

    destroy() {
        this._pill.remove_all_transitions();
        this._pill.destroy();
        this._pill = null;
        this._bars = [];
    }

    // (Re)applies the full fixed geometry at the current UI scale.
    layout() {
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
        this.place();
    }

    place() {
        const monitor = this._monitor;
        const s = this._scale;
        this._pill.set_position(
            monitor.x + Math.floor((monitor.width - PILL_W * s) / 2),
            monitor.y + monitor.height - (PILL_H + MARGIN_BOTTOM) * s);
    }

    showRecording(animate) {
        this._icon.opacity = 255;
        this._spinner.opacity = 0;
        this._pill.remove_all_transitions();
        this._pill.show();
        this.place();

        if (animate) {
            this._pill.opacity = 0;
            this._pill.ease({
                opacity: 255,
                duration: 150,
                mode: Clutter.AnimationMode.EASE_OUT_QUAD,
            });
        } else {
            this._pill.opacity = 255;
        }
    }

    showProcessing() {
        this._icon.opacity = 0;
        this._spinner.rotation_angle_z = 0;
        this._spinner.opacity = 255;
        this._pill.show();
        this.place();
    }

    setSpinnerRotation(angle) {
        this._spinner.rotation_angle_z = angle;
    }

    setBar(index, height, opacity) {
        const bar = this._bars[index];
        this._setBarHeight(bar, height);
        bar.opacity = opacity;
    }

    dismiss(animate) {
        this._pill.remove_all_transitions();
        if (animate) {
            this._pill.ease({
                opacity: 0,
                duration: 200,
                mode: Clutter.AnimationMode.EASE_IN_QUAD,
                onComplete: () => this._pill.hide(),
            });
        } else {
            this._pill.hide();
        }
    }

    hideNow() {
        this._pill.remove_all_transitions();
        this._pill.hide();
    }

    // Bars are vertically centered in the wave box; heights are in logical
    // px and clamped so a bar can never exceed the box (the box clips too).
    _setBarHeight(bar, h) {
        const s = this._scale;
        const hp = Math.round(Math.min(Math.max(h, 1), BAR_MAX) * s);
        bar.set_size(BAR_WIDTH * s, hp);
        bar.set_y(Math.round((BAR_MAX * s - hp) / 2));
    }
}

export class Overlay {
    constructor() {
        this._state = 'hidden'; // hidden | recording | processing
        this._level = 0;        // smoothed mic level 0..1
        this._target = 0;
        this._phase = 0;
        this._tick = 0;
        this._mode = 'pointer';
        this._views = [];

        this._themeContext = St.ThemeContext.get_for_stage(global.stage);
        this._scaleChangedId = this._themeContext.connect(
            'notify::scale-factor', () => this._onScaleChanged());
        this._monitorsChangedId = Main.layoutManager.connect(
            'monitors-changed', () => this._onMonitorsChanged());
    }

    destroy() {
        this._stopTick();

        if (this._scaleChangedId) {
            this._themeContext.disconnect(this._scaleChangedId);
            this._scaleChangedId = 0;
        }
        if (this._monitorsChangedId) {
            Main.layoutManager.disconnect(this._monitorsChangedId);
            this._monitorsChangedId = 0;
        }

        this._destroyViews();
        this._themeContext = null;
    }

    _animationsEnabled() {
        return St.Settings.get().enable_animations;
    }

    _onScaleChanged() {
        this._views.forEach(view => view.layout());

        // layout() resets bar geometry. Reapply the current frame immediately;
        // reduced motion has no tick that could repair it later.
        if (this._state === 'recording') {
            if (this._tick)
                this._renderRecordingFrame();
            else
                this._renderReducedRecording();
        } else if (this._state === 'processing') {
            if (this._tick)
                this._renderProcessingFrame();
            else
                this._renderReducedProcessing();
        }
    }

    _normalizeMode(mode) {
        return MONITOR_MODES.has(mode) ? mode : 'pointer';
    }

    _monitorsForMode() {
        const monitors = Main.layoutManager.monitors;

        if (this._mode === 'all')
            return monitors;

        if (this._mode === 'primary') {
            const primary = Main.layoutManager.primaryMonitor ?? monitors[0];
            return primary ? [primary] : [];
        }

        const [px, py] = global.get_pointer();
        const pointerMonitor =
            monitors.find(
                monitor => px >= monitor.x &&
                    px < monitor.x + monitor.width &&
                    py >= monitor.y &&
                    py < monitor.y + monitor.height) ??
            Main.layoutManager.primaryMonitor ??
            monitors[0];
        return pointerMonitor ? [pointerMonitor] : [];
    }

    _destroyViews() {
        this._views.forEach(view => view.destroy());
        this._views = [];
    }

    _rebuildViews() {
        const monitors = this._monitorsForMode();
        this._destroyViews();
        this._views = monitors.map(monitor => new OverlayView(monitor));
    }

    _onMonitorsChanged() {
        // Monitor objects and indices are topology snapshots. Rebuild from the
        // current list instead of trying to preserve identity across hotplug.
        if (this._state === 'hidden')
            return;

        this._rebuildViews();
        this._restoreVisibleState();
    }

    _restoreVisibleState() {
        const animations = this._animationsEnabled();

        if (this._state === 'recording') {
            this._views.forEach(view => view.showRecording(false));
            if (animations)
                this._renderRecordingFrame();
            else
                this._renderReducedRecording();
        } else if (this._state === 'processing') {
            this._views.forEach(view => view.showProcessing());
            if (animations)
                this._renderProcessingFrame();
            else
                this._renderReducedProcessing();
        }

        if (animations)
            this._startTick();
        else
            this._stopTick();
    }

    showRecording(mode = 'pointer') {
        this._state = 'recording';
        this._level = 0;
        this._target = 0;
        this._mode = this._normalizeMode(mode);

        // Rebuild for every recording: pointer mode follows the pointer's
        // current monitor, while primary/all use the latest topology snapshot.
        this._rebuildViews();

        const animations = this._animationsEnabled();
        this._views.forEach(view => view.showRecording(animations));
        if (animations) {
            this._renderRecordingFrame();
            this._startTick();
        } else {
            // Reduced motion: static pill, mid-height bars, no waveform.
            this._stopTick();
            this._renderReducedRecording();
        }
    }

    setLevel(level) {
        this._target = Math.min(Math.max(level, 0), 1);
    }

    showProcessing() {
        if (this._state === 'hidden')
            return;

        this._state = 'processing';
        this._views.forEach(view => view.showProcessing());

        if (this._animationsEnabled()) {
            this._renderProcessingFrame();
            this._startTick();
        } else {
            // Reduced motion: static spinner glyph, dimmed dots.
            this._stopTick();
            this._renderReducedProcessing();
        }
    }

    // A dictation Finished. Only dismiss when we're not already showing a
    // newer, chained recording.
    finishIfProcessing() {
        if (this._state !== 'processing')
            return;
        this.dismiss();
    }

    // Fade every view out from any state — used for Failed/Cancelled, which
    // can arrive while still 'recording' (quick tap discarded by the daemon).
    dismiss() {
        if (this._state === 'hidden')
            return;

        this._state = 'hidden';
        this._stopTick();
        const animate = this._animationsEnabled();
        this._views.forEach(view => view.dismiss(animate));
    }

    // Immediate dismissal, no fade — used when the daemon vanishes from the
    // bus mid-dictation and no terminal signal will ever arrive.
    hideNow() {
        this._state = 'hidden';
        this._stopTick();
        this._views.forEach(view => view.hideNow());
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

    _renderRecordingFrame() {
        for (let i = 0; i < N_BARS; i++) {
            const sway =
                0.35 + 0.65 * Math.abs(Math.sin(this._phase * 2.4 + i * 0.55));
            const height =
                BAR_MIN + (BAR_MAX - BAR_MIN) * this._level * sway;
            this._views.forEach(view => view.setBar(i, height, 255));
        }
    }

    _renderProcessingFrame() {
        const rotation = (this._phase * 240) % 360;
        this._views.forEach(view => view.setSpinnerRotation(rotation));
        for (let i = 0; i < N_BARS; i++) {
            const wave = 0.5 + 0.5 * Math.sin(this._phase * 4 - i * 0.45);
            const opacity = Math.round(90 + 165 * wave);
            this._views.forEach(view => view.setBar(i, BAR_MIN, opacity));
        }
    }

    _renderReducedRecording() {
        const height = (BAR_MIN + BAR_MAX) / 2;
        for (let i = 0; i < N_BARS; i++)
            this._views.forEach(view => view.setBar(i, height, 255));
    }

    _renderReducedProcessing() {
        this._views.forEach(view => view.setSpinnerRotation(0));
        for (let i = 0; i < N_BARS; i++)
            this._views.forEach(view => view.setBar(i, BAR_MIN, 160));
    }

    _onTick() {
        this._phase += TICK_MS / 1000;
        if (this._state === 'recording') {
            // Ease toward the daemon-reported level (~15 Hz) for fluid motion.
            this._level += (this._target - this._level) * 0.35;
            this._renderRecordingFrame();
        } else if (this._state === 'processing') {
            // Bars collapse to a line; a brightness wave travels through the
            // gradient while whisper works, and every spinner turns from the
            // same controller phase.
            this._renderProcessingFrame();
        }
    }
}
