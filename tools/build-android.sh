#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Builds the signed APK of the Android application (DomoComm) into dist/DomoComm-<version>.apk and prints its SHA-256.
# Needs: a JDK 17 or 21, the Android SDK (ANDROID_HOME, default ~/Android/Sdk, platform 35 and build-tools), and the
# signing key (DOMOCOMM_SIGNING, default ~/.config/domocomm/signing.properties; see docs/android.md). Nothing is
# published: the file is only written to dist/.
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "build-android: $*" >&2; exit 1; }

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Android/Sdk}"
[ -d "$ANDROID_HOME/platforms" ] || die "Android SDK not found in $ANDROID_HOME (set ANDROID_HOME, see docs/android.md)."

# Gradle 9 with this Android plugin runs on JDK 17 to 21; a newer default JDK is not used.
if [ -z "${JAVA_HOME:-}" ]; then
  for j in /usr/lib/jvm/java-21-openjdk-* /usr/lib/jvm/java-17-openjdk-*; do
    [ -x "$j/bin/java" ] && { JAVA_HOME="$j"; break; }
  done
fi
[ -n "${JAVA_HOME:-}" ] || die "no JDK 17 or 21 found (set JAVA_HOME, e.g. sudo apt install openjdk-21-jdk)."
export JAVA_HOME

SIGNING="${DOMOCOMM_SIGNING:-$HOME/.config/domocomm/signing.properties}"
[ -f "$SIGNING" ] || die "signing key missing: $SIGNING (see docs/android.md, section Signing key)."

version="$(sed -n 's/.*versionName *= *"\([^"]*\)".*/\1/p' android/app/build.gradle.kts | head -1)"
[ -n "$version" ] || die "versionName not found in android/app/build.gradle.kts."

(cd android && ./gradlew --no-daemon --console=plain assembleRelease)

src=android/app/build/outputs/apk/release/app-release.apk
[ -f "$src" ] || die "no signed APK produced ($src)."

apksigner="$(ls -d "$ANDROID_HOME"/build-tools/*/apksigner 2>/dev/null | sort -V | tail -1)"
[ -n "$apksigner" ] || die "apksigner not found in the build-tools of $ANDROID_HOME."
"$apksigner" verify --min-sdk-version 26 "$src" || die "the APK signature is not valid."

mkdir -p dist
out="dist/DomoComm-$version.apk"
cp -f "$src" "$out"
echo
echo "APK: $out"
( cd dist && sha256sum "DomoComm-$version.apk" )
echo "Signing certificate (SHA-256):"
"$apksigner" verify --print-certs "$out" | sed -n 's/.*certificate SHA-256 digest: /  /p'
