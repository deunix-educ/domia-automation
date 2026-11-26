# ADR 0066 — The Android application (DomoComm) ships in the repository, signed APK on GitHub

Status: **implemented** (2026-11-26, lots 1 and 2; the APK is published at lot 3).

## Context
The server relays SMS and calls through an Android application ([ADR 0037](0037-android-relay-and-messaging.md), [ADR 0039](0039-android-relay-server.md)). That application lived in another private project (`VideoComm`, for an older system), was not part of this repository, had no licence statement, embedded the author's personal mkcert authority and had two defects: the dot of a configured server never turned green (the service never told the screen about a connection), and the service refused to start unless all eight permissions were granted.

A user who wants the SMS relay needs the application. Without it in the repository, DomiNet's documented feature cannot be reproduced.

## Options
- **A. Keep it outside, document the protocol.** Nothing to maintain, but nobody can install the relay.
- **B. Copy the source into the repository (`android/`) and publish a signed APK as a GitHub release asset.** One place for the code and the server that speaks to it; the build stays out of `make ci`. **Chosen.**
- **C. Publish on the Play Store or F-Droid.** Reaches more people, but needs an account, policy reviews (the SMS and call-log permissions are restricted there) and a lasting commitment. Possible later, not needed now.
- **D. A web page or an existing SMS gateway application.** Another design, with another protocol: out of scope.

## Decision
Option B.
- **Source.** `android/` (Gradle, Kotlin, AGPL-3.0-or-later, comments in English, texts in English and French). Renamed `DomoComm` (application) and `DomoConf` (package `com.domoconf.comm`, which is also the application id: for Android a new application, the former `VideoComm` is uninstalled).
- **No authority built in.** The personal mkcert authority is removed. The application trusts the authorities of Android and those the user installed on the phone (`network_security_config.xml`). Clear text stays refused.
- **Backups off.** The token of a server is in the private preferences; `allowBackup` is false so that it does not reach a backup.
- **Signing.** A release key created outside the repository, read from `~/.config/domocomm/signing.properties` (or `DOMOCOMM_SIGNING`). The repository only ignores key files. The APK is built and checked by `make android-apk` (`tools/build-android.sh`: JDK 17 or 21, Android SDK, `apksigner verify`), which writes `dist/DomoComm-<version>.apk` and prints its SHA-256 and the certificate fingerprint. The fingerprint is published in the documentation so users can check what they install.
- **Publication.** A GitHub release of its own, tag `android-<version>`, independent of the server's version, with the APK and its SHA-256 in the note. Nothing is published by the build.
- **Gate.** `make ci` does not build the application (it would need the SDK on every machine and every push). The build and the Android lint are run by hand before a release.

## Consequences
- The key is a long-term secret: **without it no installed copy can be updated** (Android refuses another signature). It must be backed up outside the machine. A fork signs with its own key and is, for Android, another application.
- The application stays a thin relay: no tests of its own yet, no Play Store listing. The wire protocol is covered on the server side (`android_ws`, `android_oracle`).
- The installation from a file needs the user to allow *restricted settings* on Android 13 and later (documented in [Android relay](../android.md)); this is a platform rule, not a choice.
- The dot of each server follows the service through a listener set by the screen while it is visible; a generation counter drops the late report of a socket closed by a reload.

## As built (2026-11-26)
- Lot 1 (`4395f71`): import, renaming, dot, permissions, authority removed. Checked on a phone (GrapheneOS, Android 15): the dot turns green, the service starts with a refused permission, `wss://` over the VPN works.
- Lot 2: signing, `make android-apk`, documentation. The Android lint passes without error (33 warnings left: library versions, icon without a monochrome layer).
- **Not done**: a unit test of the application, a build in `make ci-full`, an update path tested from one version to the next.
