# QuickWhisper

**Push-to-talk voice transcription for Linux (GNOME/Wayland).** Hold a key, speak,
release — your words are transcribed locally by [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
and land in your clipboard, ready to paste. No cloud, no audio ever leaving your machine.

> **Status:** early but functional (phase 1). Hotkey → record → transcribe → clipboard +
> desktop notification works end to end. Planned next: transcription history CLI, an
> animated recording overlay, and automatic paste into the focused text field — see
> [Roadmap](#roadmap).

## How it works

```
hold F12 ──▶ record mic (PipeWire) ──▶ release ──▶ whisper.cpp (local CPU)
                                                        │
                             clipboard + notification ◀─┘
```

- A small daemon listens for the push-to-talk key and drives a synchronous
  state machine: `Idle → Recording → Processing → Idle`.
- Audio is captured via [cpal](https://github.com/rustaudio/cpal), downmixed and
  resampled to the 16 kHz mono format whisper expects.
- Transcription runs fully offline with [whisper-rs](https://github.com/tazz4843/whisper-rs)
  (whisper.cpp bindings). The model stays resident in memory, so each dictation
  only pays inference time (~0.3× real time for the `small` model on a modern CPU).
- The result is copied to the Wayland clipboard and shown in a desktop notification.

### Why evdev for the hotkey?

Wayland does not let applications grab global hotkeys. The standard answer is the
`GlobalShortcuts` XDG portal, but some distributions (e.g. Rocky Linux 10) ship a
GNOME portal backend without it. QuickWhisper therefore reads the key directly from
`/dev/input` (evdev) — the same technique Mumble uses for push-to-talk. This gives
exact press/release pairs on any session type, at the cost of requiring membership
in the `input` group. The hotkey backend is isolated in `src/hotkey.rs`, so a portal
backend can be added where available.

## Requirements

- Linux with a Wayland session (developed and tested on Rocky Linux 10 + GNOME;
  other distros/desktops should work for the clipboard flow)
- PipeWire (or plain ALSA) for audio capture
- `wl-clipboard` (the `wl-copy` binary) — usually preinstalled on GNOME
- [Rust](https://rustup.rs) toolchain to build
- Build dependencies (Fedora/RHEL/Rocky):

```bash
sudo dnf install -y cmake clang clang-devel gcc-c++ alsa-lib-devel
```

  On Debian/Ubuntu: `sudo apt install cmake clang libclang-dev build-essential libasound2-dev`

## Installation

```bash
git clone <repo-url> && cd quickwhisper
cargo install --path .

# One-time setup:
sudo usermod -aG input $USER          # allow reading the push-to-talk key (re-login required)
quickwhisper model download small     # ~466 MB, stored in ~/.local/share/quickwhisper/models
```

Log out and back in (for the `input` group), then:

```bash
quickwhisper daemon
```

Hold **F12**, speak, release. A notification shows the transcription; `Ctrl+V` pastes it.

### Run as a service (starts with your session)

```bash
mkdir -p ~/.config/systemd/user
cp data/quickwhisper.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now quickwhisper
journalctl --user -u quickwhisper -f     # logs
```

## CLI reference

| Command | Purpose |
|---|---|
| `quickwhisper daemon` | Run the push-to-talk daemon |
| `quickwhisper status` | Environment diagnostics (model, Wayland, evdev access) |
| `quickwhisper config show` | Print current configuration |
| `quickwhisper config get <key>` | Print one value |
| `quickwhisper config set <key> <value>` | Change a value (e.g. `config set language auto`) |
| `quickwhisper model download <name>` | Download a ggml model (`tiny`…`large-v3`) |
| `quickwhisper model list` | List downloaded models |
| `quickwhisper record --secs N` | Microphone test (writes a WAV) |
| `quickwhisper hotkey-test` | Show press/release events for the configured key |
| `quickwhisper transcribe <file.wav>` | Transcribe a WAV file and report timing |

## Configuration

`~/.config/quickwhisper/config.toml` (created on first run, editable via `config set`):

```toml
[hotkey]
key = "F12"            # any evdev key name: F9, PAUSE, …
mode = "push-to-talk"  # or "toggle" (press to start, press to stop)

[audio]
device = "default"     # or an exact input device name
max_recording_secs = 120

[whisper]
model = "small"        # tiny | base | small | medium | large-v3
language = "pt"        # a fixed language code, or "auto" to detect
```

**Language:** fixing the language (`pt`, `en`, …) is faster and far more reliable for
short phrases; `auto` adds a detection pass (~2× slower) and can misread brief snippets.
**Model:** `small` is a good accuracy/speed balance on CPU; `base` is ~3× faster and
noticeably less accurate.

## Privacy

Everything runs locally: audio is captured to memory, transcribed on your CPU, and
discarded. Nothing is uploaded, logged, or persisted (until the opt-in history feature
lands — which will also be local, in SQLite).

The daemon reads input devices only to detect the configured key; all other key events
are discarded immediately.

## Roadmap

Detailed architecture and phased plan live in [PLAN.md](PLAN.md) (in Portuguese):

- **Phase 2** — transcription history: SQLite store + `history list/copy/delete` CLI
- **Phase 3** — GNOME Shell extension: animated recording pill overlay (purple→orange
  waveform), automatic paste into the focused field via `Clutter.VirtualInputDevice`
- **Phase 4** — RPM packaging, multi-monitor overlay, polish

## License

MIT — see [LICENSE](LICENSE).
