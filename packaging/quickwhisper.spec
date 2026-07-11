# Built from the working tree by packaging/build-rpm.sh.
#
# Scriptlets are written out by hand (no systemd-rpm-macros dependency) so the
# spec builds on a bare rpm-build install.

# Fallbacks for hosts without systemd-rpm-macros.
%{!?_userunitdir:%global _userunitdir %{_prefix}/lib/systemd/user}
%{!?_userpresetdir:%global _userpresetdir %{_prefix}/lib/systemd/user-preset}

# cargo already produces a fully linked, optimized binary; skip debuginfo
# extraction, which chokes on some Rust build outputs.
%global debug_package %{nil}

%global extension_uuid quickwhisper@alaorneto.github.io

Name:           quickwhisper
Version:        0.1.0
Release:        4%{?dist}
Summary:        Push-to-talk voice transcription for GNOME/Wayland
License:        MIT
URL:            https://github.com/alaorneto/quickwhisper
Source0:        %{name}-%{version}.tar.gz

BuildRequires:  cmake
BuildRequires:  gcc-c++
BuildRequires:  alsa-lib-devel
# The Rust toolchain usually comes from rustup, which rpm cannot see, so it is
# checked in %%build instead of declared here.

Requires:       wl-clipboard
Requires:       gnome-shell

%description
QuickWhisper is a local push-to-talk transcription daemon for GNOME on
Wayland. Hold a hotkey, speak, release: whisper.cpp transcribes the audio
on-device and the text is pasted into the focused field (via the bundled
GNOME Shell extension) or copied to the clipboard. Includes a transcription
history CLI. On first start the daemon downloads the configured whisper
model (~500 MB for "small") and enables the Shell extension automatically.

%prep
%autosetup

%build
command -v cargo >/dev/null 2>&1 || {
    echo 'error: cargo not found — install Rust (https://rustup.rs) first' >&2
    exit 1
}
cargo build --release --locked

%install
install -Dm755 target/release/quickwhisper %{buildroot}%{_bindir}/quickwhisper
install -Dm644 data/quickwhisper.service %{buildroot}%{_userunitdir}/quickwhisper.service
install -Dm644 packaging/90-quickwhisper.preset %{buildroot}%{_userpresetdir}/90-quickwhisper.preset
mkdir -p %{buildroot}%{_datadir}/gnome-shell/extensions
cp -r extension/%{extension_uuid} %{buildroot}%{_datadir}/gnome-shell/extensions/

%post
# Fresh install: enable the daemon for every account (honors the preset) and
# start it right away for anyone logged in, so dictation works with no
# relogin and no manual steps. The daemon itself downloads the whisper model
# and enables the Shell extension on startup.
if [ "$1" -eq 1 ]; then
    systemctl --global preset quickwhisper.service >/dev/null 2>&1 || :
    for user in $(loginctl list-users --no-legend 2>/dev/null | awk '{print $2}'); do
        systemctl --machine="${user}@.host" --user daemon-reload >/dev/null 2>&1 || :
        systemctl --machine="${user}@.host" --user start quickwhisper.service >/dev/null 2>&1 || :
    done
fi

%preun
# Full removal: stop running instances and drop the global enablement.
if [ "$1" -eq 0 ]; then
    for user in $(loginctl list-users --no-legend 2>/dev/null | awk '{print $2}'); do
        systemctl --machine="${user}@.host" --user stop quickwhisper.service >/dev/null 2>&1 || :
    done
    systemctl --global disable quickwhisper.service >/dev/null 2>&1 || :
fi

%postun
# Upgrade: restart running instances so the new binary takes over.
if [ "$1" -ge 1 ]; then
    for user in $(loginctl list-users --no-legend 2>/dev/null | awk '{print $2}'); do
        systemctl --machine="${user}@.host" --user try-restart quickwhisper.service >/dev/null 2>&1 || :
    done
fi

%files
%license LICENSE
%doc README.md
%{_bindir}/quickwhisper
%{_userunitdir}/quickwhisper.service
%{_userpresetdir}/90-quickwhisper.preset
%{_datadir}/gnome-shell/extensions/%{extension_uuid}/

%changelog
* Sat Jul 11 2026 Alaor Barroso de Carvalho Neto <alaorneto@gmail.com> - 0.1.0-4
- Overlay: rebuild the pill with fully deterministic geometry — all sizes and
  positions computed in JS (scaled by the UI scale factor), stylesheet is
  paint-only, and the wave box clips to its allocation. Fixes the waveform
  rendering larger than the pill for good (the previous measure-and-lock
  approach depended on style-resolution timing).
- Overlay: show a rotating spinner in the mic icon's slot while transcribing,
  instead of leaving the space empty.

* Wed Jul 08 2026 Alaor Barroso de Carvalho Neto <alaorneto@gmail.com> - 0.1.0-3
- Overlay: fix pill locked narrower than the waveform (style was measured
  before the stylesheet applied), letting the bars spill past its right edge.

* Wed Jul 08 2026 Alaor Barroso de Carvalho Neto <alaorneto@gmail.com> - 0.1.0-2
- Discard taps shorter than 1s; overlay dismisses the pill on Cancelled/Failed
  from any state (fixes pill stuck on quick taps).
- Overlay visual refresh; fix pill shape (border-radius exceeded half height).

* Tue Jul 07 2026 Alaor Barroso de Carvalho Neto <alaorneto@gmail.com> - 0.1.0-1
- Initial package: daemon + CLI, GNOME Shell extension, user service with
  autostart preset, immediate start for logged-in users on install.
