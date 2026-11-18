#!/bin/sh
# Builds Portman.app (menu bar only, no Dock icon) and installs it into ~/Applications.
set -eu

cd "$(dirname "$0")"
cargo build --release

app="${1:-$HOME/Applications/Portman.app}"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/portman-menubar "$app/Contents/MacOS/portman-menubar"

cat > "$app/Contents/Info.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Portman</string>
  <key>CFBundleDisplayName</key><string>Portman</string>
  <key>CFBundleIdentifier</key><string>com.camcurry20.portman-menubar</string>
  <key>CFBundleExecutable</key><string>portman-menubar</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>LSUIElement</key><true/>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign - "$app" >/dev/null 2>&1 || true
echo "Installed $app"
