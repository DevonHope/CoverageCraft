#!/bin/sh
# Build RadioCraft (release) and install it as RadioCraft.app, by default in ~/Applications.
#
#   scripts/install-macos.sh           build and install now
#   scripts/install-macos.sh --hooks   also add local git hooks that reinstall it in the
#                                      background after every commit, merge, pull or checkout
#
# Environment: RADIOCRAFT_APP_DIR (where the app goes), RADIOCRAFT_JOBS (cargo -j, default 2).
# The build runs at low priority. The app is replaced in one step; a running copy keeps going
# until you quit it, and the next launch is the new build.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
DEST=${RADIOCRAFT_APP_DIR:-$HOME/Applications}
JOBS=${RADIOCRAFT_JOBS:-2}
cd "$ROOT"

if [ "${1:-}" = "--hooks" ]; then
    HOOKS=$(git rev-parse --git-path hooks)
    mkdir -p "$HOOKS"
    for hook in post-commit post-merge post-checkout post-rewrite; do
        cat > "$HOOKS/$hook" <<'HOOK'
#!/bin/sh
# Local only (not in the repository): reinstall RadioCraft.app in the background.
# A checkout of single files ($3 = 0) changes nothing to build.
[ "$(basename "$0")" = "post-checkout" ] && [ "${3:-1}" = "0" ] && exit 0
ROOT=$(git rev-parse --show-toplevel)
LOG="$HOME/Library/Logs/RadioCraft/install.log"
mkdir -p "$(dirname "$LOG")"
# One install at a time: later ones wait their turn.
nohup /usr/bin/lockf -k "${TMPDIR:-/tmp}/radiocraft-install.lock" \
    "$ROOT/scripts/install-macos.sh" >>"$LOG" 2>&1 </dev/null &
HOOK
        chmod +x "$HOOKS/$hook"
    done
    echo "git hooks added: RadioCraft.app is reinstalled after each commit (log: ~/Library/Logs/RadioCraft/install.log)"
fi

COMMIT=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
echo "== $(date '+%Y-%m-%d %H:%M:%S') building RadioCraft at $COMMIT"
nice -n 19 cargo build --release -j "$JOBS" -p radiocraft -p radiocraft-cli

TARGET=$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')
BIN="$TARGET/release"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)

mkdir -p "$DEST"
NEW="$DEST/.RadioCraft.app.new"
rm -rf "$NEW"
mkdir -p "$NEW/Contents/MacOS" "$NEW/Contents/Resources"
cp "$BIN/radiocraft" "$BIN/radiocraft-cli" "$NEW/Contents/MacOS/"
echo "$COMMIT" > "$NEW/Contents/Resources/commit.txt"
# The app icon: the logo drawn in VectorCraft (assets/icon.vectorcraft), exported as
# assets/icon-1024.png, at each size macOS wants.
ICONSET=$(mktemp -d)/RadioCraft.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    sips -z "$s" "$s" assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
    sips -z "$((s * 2))" "$((s * 2))" assets/icon-1024.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$NEW/Contents/Resources/RadioCraft.icns"
rm -rf "$(dirname "$ICONSET")"
cat > "$NEW/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>RadioCraft</string>
    <key>CFBundleDisplayName</key><string>RadioCraft</string>
    <key>CFBundleIdentifier</key><string>local.radiocraft.beta</string>
    <key>CFBundleExecutable</key><string>radiocraft</string>
    <key>CFBundleIconFile</key><string>RadioCraft</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$COMMIT</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
# A local build is signed ad hoc, so macOS runs it on this machine.
codesign --force --deep --sign - "$NEW" >/dev/null 2>&1 || true
rm -rf "$DEST/RadioCraft.app"
mv "$NEW" "$DEST/RadioCraft.app"
# Tell Finder and the Dock the app (and its icon) changed.
touch "$DEST/RadioCraft.app"
echo "== installed $DEST/RadioCraft.app ($VERSION, $COMMIT)"
if command -v osascript >/dev/null 2>&1; then
    osascript -e "display notification \"Build $COMMIT installed\" with title \"RadioCraft updated\"" >/dev/null 2>&1 || true
fi
