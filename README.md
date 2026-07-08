# QuickWhisper

**Push-to-talk voice transcription for Linux (GNOME/Wayland).** Hold a key, speak,
release — your words are transcribed locally by [whisper.cpp](https://github.com/ggerganov/whisper.cpp)
and land in your clipboard, ready to paste. No cloud, no audio ever leaving your machine.

> **Status:** functional (phase 3). Hold the key and a pill overlay with a
> voice-reactive waveform appears at the bottom of the screen; release, and when
> transcription finishes the text is pasted into the focused field and copied to the
> clipboard. Chained dictations are supported. Without the GNOME extension, it degrades
> gracefully to clipboard + notifications. See [Roadmap](#roadmap).

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
- The result is copied to the Wayland clipboard; with the companion GNOME Shell
  extension it is also **pasted into the focused text field** automatically.
- The extension renders the recording overlay: a pill at the bottom of the screen
  with a purple→orange waveform that reacts to your voice (daemon streams the mic
  RMS over D-Bus), turning into a traveling shimmer while whisper processes.
  It honors the system's reduced-motion (animations) setting.
- Daemon and extension talk over the session D-Bus (`io.github.alaor.QuickWhisper`);
  without the extension, the daemon falls back to desktop notifications.
- On startup the daemon downloads the configured whisper model if missing and asks
  GNOME Shell to enable the companion extension, so a fresh install needs no manual
  bootstrap.

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

### RPM (recommended on Fedora/RHEL/Rocky)

```bash
sudo dnf install -y rpm-build cmake clang clang-devel gcc-c++ alsa-lib-devel
git clone <repo-url> && cd quickwhisper
./packaging/build-rpm.sh
sudo dnf install target/rpmbuild/RPMS/x86_64/quickwhisper-*.rpm
```

The package installs the binary, the GNOME Shell extension (system-wide) and a
systemd **user** service enabled for every account via preset. The post-install
script starts the daemon immediately for logged-in users; on its first start the
daemon downloads the whisper model (~466 MB for `small`) and enables the Shell
extension by itself.

Two things still need one action from you:

```bash
sudo usermod -aG input $USER   # allow reading the push-to-talk key
```

then **log out and back in** — this applies the `input` group and lets GNOME Shell
load the freshly installed extension (Wayland cannot reload the Shell in place).
After that, hold **F12**, speak, release: the recording pill appears and the text is
pasted into the focused field. Note: synthetic `Ctrl+V` pastes into most apps;
terminals typically use `Ctrl+Shift+V`, so there the text stays in the clipboard.

**Updating:** pull the new code, re-run `./packaging/build-rpm.sh`, then
`sudo dnf upgrade target/rpmbuild/RPMS/x86_64/quickwhisper-*.rpm`. The daemon is
restarted automatically; if the update touches the GNOME Shell extension, log out
and back in for it to take effect.

### From source

```bash
git clone <repo-url> && cd quickwhisper
cargo install --path .

sudo usermod -aG input $USER          # allow reading the push-to-talk key (re-login required)
quickwhisper model download small     # optional: the daemon also fetches it on first start
```

Log out and back in (for the `input` group), then:

```bash
quickwhisper daemon
```

Hold **F12**, speak, release. A notification shows the transcription; `Ctrl+V` pastes it.

#### GNOME Shell extension (overlay + auto-paste)

```bash
ln -sfn "$PWD/extension/quickwhisper@alaorneto.github.io" \
    ~/.local/share/gnome-shell/extensions/quickwhisper@alaorneto.github.io
gnome-extensions enable quickwhisper@alaorneto.github.io
```

Log out and back in — the pill overlay and automatic paste become active.

#### Run as a service (starts with your session)

```bash
sudo install -Dm644 data/quickwhisper.service /etc/systemd/user/quickwhisper.service
systemctl --user edit quickwhisper        # override ExecStart with your binary path
systemctl --user daemon-reload
systemctl --user enable --now quickwhisper
journalctl --user -u quickwhisper -f      # logs
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
| `quickwhisper history list [--limit N] [--json]` | Past transcriptions, newest first |
| `quickwhisper history show <id>` | Full text of one transcription |
| `quickwhisper history copy <id>` | Copy a past transcription to the clipboard |
| `quickwhisper history delete <id>…` | Delete transcriptions |
| `quickwhisper history clear [--yes]` | Delete the whole history |
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
discarded — only the resulting *text* is kept, in a local SQLite history
(`~/.local/share/quickwhisper/history.db`) that you can inspect and prune with
`quickwhisper history`. Nothing is ever uploaded.

The daemon reads input devices only to detect the configured key; all other key events
are discarded immediately.

## Roadmap

Detailed architecture and phased plan live in [PLAN.md](PLAN.md) (in Portuguese):

- ~~**Phase 2** — transcription history: SQLite store + `history` CLI~~ ✔ done
- ~~**Phase 3** — GNOME Shell extension: animated recording pill overlay, automatic
  paste into the focused field via `Clutter.VirtualInputDevice`~~ ✔ done
- **Phase 4** — ~~RPM packaging~~ ✔, multi-monitor overlay, cancel gesture, polish

## License

MIT — see [LICENSE](LICENSE).
