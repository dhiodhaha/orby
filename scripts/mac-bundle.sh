#!/bin/sh
# Builds Orby for macOS and installs it as ~/Applications/Orby.app: an accessory app (no Dock
# icon, LSUIElement), ad-hoc signed. Run on the Mac from the repo:
#
#   scripts/mac-bundle.sh            # debug build (smaller target folder, quicker)
#   scripts/mac-bundle.sh release    # optimised build
#
# Then start it with `open ~/Applications/Orby.app`. A running Orby is quit first.
set -eu

profile=${1:-debug}
cd "$(dirname "$0")/.."

if [ "$profile" = release ]; then
    cargo build -p orb-proto --release
else
    cargo build -p orb-proto
fi

app="$HOME/Applications/Orby.app"
pkill -x Orby 2>/dev/null || true
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "target/$profile/orb-proto" "$app/Contents/MacOS/Orby"

cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Orby</string>
    <key>CFBundleExecutable</key>
    <string>Orby</string>
    <key>CFBundleIdentifier</key>
    <string>dev.dhio.orby</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Orby</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>13.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$app"
echo "installed $app ($profile)"
