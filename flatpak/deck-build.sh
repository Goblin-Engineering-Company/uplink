#!/usr/bin/env bash
# deck-build.sh — build + install GEC Uplink as a FLATPAK on the Steam Deck.
#
# Uses the .deb (NOT the AppImage): the AppImage builder patches the binary's
# interpreter so it only runs via its own bundled loader — wrapped in a Flatpak
# it segfaults at _start. The .deb ships the clean, un-patched binary that runs
# against the GNOME 46 runtime (webkit2gtk 2.44 — renders on SteamOS).
#
# RUN ON THE DECK (Desktop Mode → Konsole):
#   1. Download the dev .deb from goblineng.co/download (your dev login) to ~/Downloads
#   2. bash deck-build.sh                 # auto-finds ~/Downloads/*uplink*.deb
#      (or) bash deck-build.sh /path/to/gec-uplink_0.2.0-dev.7_amd64.deb
#
# First run installs org.gnome.{Platform,Sdk}//46 + flatpak-builder (one-time).
set -euo pipefail

ID="com.goblinengineering.uplink"
RUNTIME_VER="46"
# Under $HOME (not /tmp): the sandboxed flatpak-builder can't read /tmp.
WORK="$HOME/gec-uplink-flatpak-build"

# ── locate the .deb ──
DEB="${1:-}"
[ -n "$DEB" ] || DEB="$(ls -t "$HOME"/Downloads/*.deb 2>/dev/null | grep -i uplink | head -1 || true)"
[ -n "$DEB" ] || DEB="$(ls -t "$HOME"/Downloads/*.deb 2>/dev/null | head -1 || true)"
[ -n "$DEB" ] && [ -f "$DEB" ] || {
  echo "✗ no .deb found. Download the dev .deb from goblineng.co/download to ~/Downloads," >&2
  echo "  or pass its path: bash deck-build.sh /path/to/gec-uplink_*.deb" >&2
  exit 1
}
DEB="$(readlink -f "$DEB")"
echo "== packaging: $DEB =="

# ── ensure flathub + runtime/SDK + flatpak-builder (one-time) ──
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
echo "== ensuring org.gnome.Platform//$RUNTIME_VER + Sdk + flatpak-builder (one-time) =="
flatpak install --user -y flathub "org.gnome.Platform//$RUNTIME_VER" "org.gnome.Sdk//$RUNTIME_VER" org.flatpak.Builder 2>&1 | tail -2 || true
FB="flatpak run org.flatpak.Builder"
command -v flatpak-builder >/dev/null 2>&1 && FB="flatpak-builder"

# ── extract the .deb → payload/ (clean, un-patched binary) ──
rm -rf "$WORK"; mkdir -p "$WORK/payload"; cd "$WORK"
cp "$DEB" pkg.deb
ar x pkg.deb                                   # → debian-binary, control.tar.*, data.tar.*
DATA="$(ls data.tar.* | head -1)"
tar -C payload -xf "$DATA"
[ -d payload/usr/bin ] || { echo "✗ .deb has no usr/bin — unexpected layout" >&2; exit 1; }
BIN="$(ls payload/usr/bin | head -1)"
echo "== binary: $BIN =="

# ── generate the manifest (binary only; runtime provides all libs) ──
cat > "$ID.yml" <<YML
id: $ID
runtime: org.gnome.Platform
runtime-version: '$RUNTIME_VER'
sdk: org.gnome.Sdk
command: $BIN
finish-args:
  - --socket=wayland
  - --socket=fallback-x11
  - --device=dri
  - --share=ipc
  - --share=network
  - --filesystem=host          # reach WoW AddOns anywhere (incl. Proton compatdata, microSD)
  - --talk-name=org.kde.StatusNotifierWatcher
  - --filesystem=xdg-run/tray-icon:create
modules:
  - name: gec-uplink
    buildsystem: simple
    sources:
      - type: dir
        path: payload
    build-commands:
      # ONLY the binary + desktop + icon. Do NOT copy usr/lib (the .deb's
      # usr/lib is app resources only, but Tauri embeds the frontend, so
      # there's nothing to copy — the runtime provides webkit/gtk/glib).
      - install -Dm755 "usr/bin/$BIN" "/app/bin/$BIN"
      - 'D=\$(ls usr/share/applications/*.desktop 2>/dev/null | head -1); if [ -n "\$D" ]; then sed -i "s/^Icon=.*/Icon=$ID/" "\$D"; install -Dm644 "\$D" "/app/share/applications/$ID.desktop"; fi'
      - 'for px in 32 128 256; do I=\$(ls usr/share/icons/hicolor/\${px}x\${px}/apps/*.png 2>/dev/null | head -1); [ -n "\$I" ] && install -Dm644 "\$I" "/app/share/icons/hicolor/\${px}x\${px}/apps/$ID.png"; done; true'
YML

# ── build + install (absolute paths for the sandboxed builder) ──
echo "== flatpak-builder (build + install) =="
$FB --user --install --force-clean --disable-cache "$WORK/build-dir" "$WORK/$ID.yml"

echo
echo "✓ installed. Launch it:  flatpak run $ID"
