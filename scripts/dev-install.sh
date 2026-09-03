#!/bin/bash
#
# Build DBFlux and drop it into the local macOS app bundle.
#
# The development loop this exists for: change something, run this, launch the
# app, see the change. Release builds are deliberately not the default — they
# take several minutes each and CI produces them for every platform on a tag
# anyway. A debug build is what you want while iterating: slower to run, but
# incremental rebuilds finish in well under a minute.
#
# Usage:
#   scripts/dev-install.sh             # debug build, then install
#   scripts/dev-install.sh --release   # optimized build, then install
#   scripts/dev-install.sh --no-build  # install whatever was built last
#
# Environment:
#   DBFLUX_APP   Bundle to install into (default: /Applications/DBFlux Local.app)
#
# The bundle itself is not created here — it carries its own Info.plist and
# icon, and only the executable and the version string are replaced. The
# executable is swapped with `mv` so a running instance keeps its old image
# intact until you quit and relaunch; the new one is never half-written.

set -euo pipefail

FEATURES="sqlite,postgres,mysql,mssql,mongodb,redis,dynamodb,cloudwatch,influxdb,redshift,s3,aws"
APP="${DBFLUX_APP:-/Applications/DBFlux Local.app}"
BUNDLE_ID="dev.dbflux.local"

profile="debug"
build=1
for arg in "$@"; do
  case "$arg" in
    --release)  profile="release" ;;
    --no-build) build=0 ;;
    -h|--help)  sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

# `cargo` lives in ~/.cargo/bin, which a fresh shell may not have on PATH.
export PATH="$HOME/.cargo/bin:$PATH"

repo="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo"

if [[ ! -d "$APP/Contents/MacOS" ]]; then
  echo "error: no app bundle at '$APP' — set DBFLUX_APP or create the bundle first" >&2
  exit 1
fi

if (( build )); then
  echo "==> cargo build ($profile)"
  if [[ "$profile" == "release" ]]; then
    cargo build -p dbflux --release --features "$FEATURES"
  else
    cargo build -p dbflux --features "$FEATURES"
  fi
fi

binary="target/$profile/dbflux"
if [[ ! -x "$binary" ]]; then
  echo "error: $binary does not exist — build first or drop --no-build" >&2
  exit 1
fi

echo "==> install $binary -> $APP"
# Copy to a sibling temp name, then mv: the swap is atomic, and a running
# instance is unaffected until relaunch.
cp "$binary" "$APP/Contents/MacOS/.dbflux.new"
mv -f "$APP/Contents/MacOS/.dbflux.new" "$APP/Contents/MacOS/dbflux"

# Keep Finder and About honest about which build this is.
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
if [[ -n "$version" ]]; then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "$APP/Contents/Info.plist" >/dev/null
  /usr/libexec/PlistBuddy -c "Set :CFBundleVersion $version" "$APP/Contents/Info.plist" >/dev/null
fi

echo "==> codesign (ad-hoc)"
codesign --force --sign - --identifier "$BUNDLE_ID" "$APP"
codesign --verify --deep --strict "$APP"

echo
echo "installed $profile build $version into $APP"
if pgrep -qf "$APP/Contents/MacOS/dbflux"; then
  echo "DBFlux is running — quit and relaunch to pick up the new build."
fi
