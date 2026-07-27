# Lume Mobile

Lume Mobile uses the same application in three forms:

- an installable browser PWA published as static files on GitHub Pages;
- an Android Capacitor shell;
- an iOS Capacitor shell.

No cloud relay carries agent data. The phone and computer must be on the same local network.

## Pairing

1. In desktop Lume, open **Settings → Mobile access**.
2. Enable the local network gateway.
3. Scan the one-time QR code.
4. Allow local-network access when the browser requests it.
5. Continue in the PWA, or install/open `Lume-Mobile.apk` from the same page.
6. Grant optional Prompt, Approvals, or Stop agents scopes from the desktop.

The QR secret is kept in the URL fragment, so it is never sent to the static PWA host. Pairing and API payloads use AES-256-GCM over the local connection; raw access tokens are returned once to the phone and only their SHA-256 hashes are stored on the computer. Pairing codes expire after five minutes and are single-use.

## Sync native projects

```bash
npm run mobile:sync
```

## Android

Open the generated project with:

```bash
npm run mobile:android
```

Build and signing require Android Studio, the Android SDK, and a release keystore. The local HTTP connection is only the transport carrier: pairing, tokens, commands, and responses remain authenticated and encrypted by the Lume protocol, so no user-installed certificate is required.

The app checks the signed `mobile-latest.json` manifest every six hours. Downloads are accepted only from the official Lume GitHub release path, verified with SHA-256, checked for the same package and signing certificate, and then handed to the Android system installer for confirmation.

## iOS

Open the generated project on macOS with:

```bash
npm run mobile:ios
```

Building and signing require macOS, Xcode, and an Apple Developer identity. The project includes the local-network privacy description required to reach the desktop gateway.
