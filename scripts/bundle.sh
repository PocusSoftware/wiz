#!/bin/sh
# Builds Wiz.app (release binary, icon, ad-hoc signature) and installs it to
# /Applications, replacing the previous build. The staging copy in dist/ is removed so
# Launchpad and Spotlight only see the installed app.
set -e
cd "$(dirname "$0")/.."
cargo build --release -p ssh-app
APP=dist/Wiz.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/wiz "$APP/Contents/MacOS/wiz"
cp crates/app/assets/brand/Wiz.icns "$APP/Contents/Resources/Wiz.icns"
VERSION=$(scripts/version.sh)
# Build number: commits on the current branch, so every bundle gets a higher one.
BUILD=$(git rev-list --count HEAD 2>/dev/null || echo 1)
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Wiz</string>
  <key>CFBundleDisplayName</key><string>Wiz</string>
  <key>CFBundleIdentifier</key><string>dev.enty.wiz</string>
  <key>CFBundleExecutable</key><string>wiz</string>
  <key>CFBundleIconFile</key><string>Wiz</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$BUILD</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
</dict>
</plist>
PLIST
# Sign with the first code signing certificate when there is one. An ad-hoc signature
# names the exact build, so the Keychain would forget "Always Allow" on every reinstall.
IDENTITY=$(security find-identity -v -p codesigning | awk '/\)/ {print $2; exit}')
codesign --force --deep -s "${IDENTITY:--}" "$APP"
echo "built $APP ($VERSION, build $BUILD)"
rm -rf /Applications/Wiz.app
ditto "$APP" /Applications/Wiz.app
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister
"$LSREGISTER" -u "$APP" 2>/dev/null || true
rm -rf "$APP"
"$LSREGISTER" -f /Applications/Wiz.app
echo "installed /Applications/Wiz.app"
