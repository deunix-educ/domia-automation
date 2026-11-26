# Android relay

R5 lot 3, [ADR 0039](adr/0039-android-relay-server.md) (model: [ADR 0037](adr/0037-android-relay-and-messaging.md)). The Rust server speaks exactly the protocol of the application. The application itself ships in this repository (`android/`), see [The application](#the-application-domocomm) below ([ADR 0066](adr/0066-android-application.md)).

## What the app does

It connects to `wss://<server>/ws/android/device/<token>/` (the device token, shown only once when created or regenerated) and reconnects by itself after 5 s.

| Direction | Frame | Effect |
|---|---|---|
| app → server | `sms_received {from, body, contact_name, timestamp}` | SMS stored (unread), shown live |
| app → server | `call_event {state, number, contact_name, timestamp, duration}` | call log (`ringing` / `outgoing` create, `ended` / `missed` close, `answered` changes nothing) |
| app → server | `sms_sent_ack {message_id, success, error}` | the requested SMS is "sent" or "failed" |
| server → app | `auth_ok {device_name}` | connection accepted; the device is "online" |
| server → app | `send_sms {to, body, message_id}` | sends an SMS; the app answers with `sms_sent_ack` carrying the same `message_id` |
| server → app | `pong`, `alert_notification {…}` | (the current app ignores the latter) |

Unknown token: close with code **4001**.

## Presence and heartbeat

The server sends a control frame every 30 s; with nothing received for 90 s it closes (code 4408). A new connection from the same device replaces the old one (4009). At startup every device goes "offline".

## SMS to send

`AndroidRelay::send_sms` queues the SMS (`sms_outbox`) and sends it to the device; the ack settles it. **Without an ack after 60 s the SMS fails** ("no ack from the device"). An ack from another device, for an unknown id, or late, has no effect.

## For browsers

Topic `android:<id>` of the notifications WebSocket (owner or administrator): `device_status`, `sms_received`, `call_event`, `sms_sent_ack`. The current presence arrives on subscription.

## The pages (lot 4, [ADR 0040](adr/0040-android-pages.md))

| Page | Content |
|---|---|
| `/android` | devices (live presence), last 10 SMS and calls, unread SMS count |
| `/android/devices` | add a device (name, number), **new token**, delete (with confirmation); **the token is only shown at creation or regeneration** |
| `/android/sms?device=N` | the last 100 SMS (received ones are marked read when displayed), send an SMS; each outgoing SMS shows its state: waiting, sent, failed and why |
| `/android/calls?device=N` | the last 100 calls (incoming, outgoing, missed, duration `mm:ss`, "ongoing") |

Each account sees its devices, an administrator sees all devices. Everything updates live (received SMS, calls, acks, presence). Address to enter in the app: `ws://server-address/ws/android/device/TOKEN/` (`wss://` behind HTTPS).

## Security

- The token is only stored hashed (SHA-256) and **appears in no log** (the path is masked).
- Frames limited to 64 KiB; fields clipped to the database bounds; invalid JSON ignored without closing the line.

## Tests

`cargo test -p dominet --test android_ws` (15: token, presence, replacement, heartbeat, SMS, calls, acks, messaging, timeout, alerts, subscription rights), `--test android_oracle` (**21 sessions replayed against frozen reference exchanges**), `-p dominet-domain --test android_fixtures` (frames from the Kotlin code), `--test ui android` (rights, token, sending), E2E `android` (simulated phone `tools/sim_android.py`).

## The application (DomoComm)

DomoComm is the Android application that relays the SMS and the calls of a phone to DomiNet. Its source is in `android/` (Kotlin, package `com.domoconf.comm`, same licence as the rest). It needs a phone with a SIM card and Android 8.0 or later.

### Install it

1. Download `DomoComm-<version>.apk` from the project's GitHub release page (tags `android-<version>`). Check it before installing: the release note gives its SHA-256 (`sha256sum DomoComm-<version>.apk`) and the fingerprint of the signing certificate, which stays the same for every version:
   `77:AE:DB:BD:B3:ED:C7:A9:90:E6:32:29:0C:A8:63:BA:43:55:B3:C9:C3:6C:A9:86:A0:DE:7B:5A:E8:1E:EB:F2`
2. Open the file on the phone and allow your browser or file manager to install applications once. Updating later keeps the settings, as long as the APK is signed with the same key.
3. **Android 13 and later: allow restricted settings.** An application installed from a file cannot receive the SMS and call permissions until you allow it: *Settings, Apps, DomoComm, menu (three dots), Allow restricted settings*. The menu appears after a first refused permission.
4. Open DomoComm, tap **+**, give a name and the URL of the device (below), then **Start** and accept the permissions.

### The URL

On the **Android, Devices** page of DomiNet, create a device: its token is shown **once**. The URL to enter is `wss://<server>/ws/android/device/<token>/` (`<server>` is the name or the address you open DomiNet with, `dominet.local` or the address of the VPN). A plain `ws://` is refused by Android unless the server is reached without TLS.

### Trust the certificate of your server

DomoComm trusts the authorities of Android and **those you installed on the phone**; it carries none. With the local authority made by `make certs` (mkcert), download `https://<server>/dominet-ca.crt` and install it: *Settings, Security, Encryption and credentials, Install a certificate, CA certificate*. The certificate of the server must contain the name or the address in the URL (`CERT_EXTRA_SANS` for a VPN address, then `make certs`, which also reloads nginx).

### What you see

- The dot next to the status: green while the service runs.
- The dot of each server: **green while that server is connected**, grey otherwise. It follows a loss or a return of the network.
- If a permission was refused, the service starts anyway and lists what is missing (the phone, call log and contacts permissions only limit the calls and the names). Without any SMS permission it does not start.
- The notification says how many servers are connected.

### Build it yourself

`make android-apk` builds and signs `dist/DomoComm-<version>.apk` and prints its SHA-256 and the certificate fingerprint. It needs a JDK 17 or 21, the Android SDK (`ANDROID_HOME`, default `~/Android/Sdk`, with platform 35 and build-tools) and a signing key. Nothing is published by this command. `make ci` does not build the application.

For a debug build without a key: `cd android && ./gradlew assembleDebug` (installable for tests, not meant for distribution).

### Signing key

The key is **never in the repository**. `make android-apk` reads `~/.config/domocomm/signing.properties` (or the file named by `DOMOCOMM_SIGNING`) with the lines `storeFile`, `storePassword`, `keyAlias`, `keyPassword`. To create one:

```
mkdir -p ~/.config/domocomm && chmod 700 ~/.config/domocomm
keytool -genkeypair -keystore ~/.config/domocomm/domocomm-release.jks -storetype PKCS12 \
  -alias domocomm -keyalg RSA -keysize 4096 -validity 10950 -dname "CN=DomoComm, O=DomiNet"
```

Then write the properties file with the password you typed, and `chmod 600` both files. **Back the key up outside the machine**: without it nobody can update the application installed on the phones (Android refuses an update signed by another key), and a new key means installing a new application. Anyone who forks the project signs with their own key: their APK is a different application to Android.

### Limits

- The application does not store anything outside the list of servers (name and URL with token) in its private preferences; Android backups are disabled so that the token does not leave the phone.
- The calls to send an SMS go through the default SMS manager of the phone: the operator's rules and limits apply.
