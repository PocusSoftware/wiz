#!/bin/sh
# Builds dist/Bastion.app (release binary, icon, ad-hoc signature).
set -e
cd "$(dirname "$0")/.."
cargo build --release -p ssh-app
APP=dist/Bastion.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/bastion "$APP/Contents/MacOS/bastion"
cp crates/app/assets/brand/Bastion.icns "$APP/Contents/Resources/Bastion.icns"
VERSION=$(scripts/version.sh)
# Build number: commits on the current branch, so every bundle gets a higher one.
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Bastion</string>
  <key>CFBundleDisplayName</key><string>Bastion</string>
  <key>CFBundleIdentifier</key><string>dev.enty.bastion</string>
  <key>CFBundleExecutable</key><string>bastion</string>
  <key>CFBundleIconFile</key><string>Bastion</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST
codesign --force --deep -s - "$APP"
echo "built $APP ($VERSION, build $BUILD)"
