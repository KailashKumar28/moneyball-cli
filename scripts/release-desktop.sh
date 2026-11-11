#!/bin/zsh
# Build Moneyball.app and its DMG, signed and notarized when the
# credentials are present, and prove it. Without them it builds an ad-hoc
# signed app that runs on this Mac only, and says so.
#
# Needs for a release that opens on other Macs:
#   - a "Developer ID Application" certificate in the login keychain
#   - APPLE_API_ISSUER, APPLE_API_KEY (key id), APPLE_API_KEY_PATH (.p8)
# Pass --release to fail instead of falling back to ad-hoc.
#
# The DMG's icon layout is done by scripting Finder, so the terminal this
# runs in needs Automation permission for Finder (System Settings >
# Privacy & Security > Automation); without it the build stops with
# "AppleEvent timed out (-1712)". CI=true skips the layout step and
# builds a plain DMG.
set -euo pipefail

root=${0:A:h:h}
crate=$root/crates/moneyball-desktop
want_release=${1:-}

identity=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
have_api=0
[[ -n ${APPLE_API_ISSUER:-} && -n ${APPLE_API_KEY:-} && -n ${APPLE_API_KEY_PATH:-} ]] && have_api=1

if [[ -n $identity && $have_api == 1 ]]; then
  mode=release
  export APPLE_SIGNING_IDENTITY=$identity
elif [[ $want_release == --release ]]; then
  print -u2 "release-desktop: no Developer ID certificate or no APPLE_API_* variables; refusing --release"
  exit 1
else
  mode=adhoc
  export APPLE_SIGNING_IDENTITY=-
fi

(cd $crate/ui && pnpm install --frozen-lockfile && pnpm build)
(cd $crate && ./ui/node_modules/.bin/tauri build --bundles app,dmg)

app=$root/target/release/bundle/macos/Moneyball.app
dmg=($root/target/release/bundle/dmg/Moneyball_*.dmg(om[1]))
codesign --verify --deep --strict $app

if [[ $mode == adhoc ]]; then
  print "release-desktop: AD-HOC build, NOT notarized. It runs on this Mac only."
  print "  app: $app"
  print "  dmg: $dmg"
  exit 0
fi

# Tauri notarizes and staples the .app but only signs the DMG.
xcrun notarytool submit $dmg --key $APPLE_API_KEY_PATH --key-id $APPLE_API_KEY \
  --issuer $APPLE_API_ISSUER --wait
xcrun stapler staple $dmg

# A missing credential makes Tauri skip notarization with a log line, so
# every claim is checked here; any failure stops the script.
codesign -dv --verbose=4 $app 2>&1 | grep -q 'flags=.*runtime'
spctl -a -vvv -t exec $app 2>&1 | grep -q 'Notarized Developer ID'
spctl -a -vvv -t open --context context:primary-signature $dmg 2>&1 | grep -q 'Notarized Developer ID'
xcrun stapler validate $app
xcrun stapler validate $dmg
print "release-desktop: signed, notarized and stapled."
print "  dmg: $dmg"
