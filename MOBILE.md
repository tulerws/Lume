# Lume Mobile

Lume Mobile uses the same local PWA in three forms:

- an installable browser PWA served by the desktop gateway;
- an Android Capacitor shell;
- an iOS Capacitor shell.

No cloud relay is used. The phone and computer must be on the same local network.

## Pairing

1. In desktop Lume, open **Settings → Mobile access**.
2. Enable the local network gateway.
3. Open the certificate URL on the phone and trust the Lume Local CA.
4. On Android, install `Lume-Mobile.apk` from the link shown by Lume.
5. Scan the one-time QR code, or enter the HTTPS gateway and code in the native app.
6. Grant optional Prompt, Approvals, or Stop agents scopes from the desktop.

Raw access tokens are returned once to the phone and only their SHA-256 hashes are stored on the computer. Pairing codes expire after two minutes and are single-use.

## Sync native projects

```bash
npm run mobile:sync
```

## Android

Open the generated project with:

```bash
npm run mobile:android
```

Build and signing require Android Studio, the Android SDK, and a release keystore. The Android shell trusts user-installed certificates so it can connect to the local Lume CA, while cleartext HTTP remains disabled.

The app checks the signed `mobile-latest.json` manifest every six hours. Downloads are accepted only from the official Lume GitHub release path, verified with SHA-256, checked for the same package and signing certificate, and then handed to the Android system installer for confirmation.

## iOS

Open the generated project on macOS with:

```bash
npm run mobile:ios
```

Building and signing require macOS, Xcode, and an Apple Developer identity. The project includes the local-network privacy description required to reach the desktop gateway.
