# Development guide

Build, test, and benchmark Sockspring locally. For an introduction and setup instructions, see the [README](../README.md).

## Build

Requires JDK 17 or 21, Rust/Cargo, Android SDK command-line tools, and Git. Set
`JAVA_HOME` and `ANDROID_HOME` for your installation. On macOS the Homebrew
command-line tools SDK normally lives at
`/opt/homebrew/share/android-commandlinetools`.

```bash
brew install --cask android-commandlinetools   # macOS, if needed
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
cargo install cargo-ndk --version 4.1.2 --locked
sdkmanager 'platforms;android-37.0' 'build-tools;36.0.0' 'ndk;26.3.11579264'
./gradlew assembleDebug
```

Output: `app/build/outputs/apk/debug/app-debug.apk`. The build packages Rust release
libraries for arm64-v8a, armeabi-v7a, x86, and x86_64, with 16 KiB page alignment.
Gradle runs `scripts/build-rust.sh` automatically. Cargo dependencies are locked.
Debug builds use the standard local Android debug key. Release builds use only
`upload-signing.properties` (gitignored), and are unsigned when it is absent.
See [Play publishing materials](../distribution/play/README.md) for signing and submission.

The UI pins Material 3 `1.5.0-rc01` for the public Expressive APIs. This requires
compile SDK 37 and AGP 9.1.1. The app targets SDK 37 and still supports API 24.

## Verification

```bash
cargo test --manifest-path native/Cargo.toml --locked
cargo clippy --manifest-path native/Cargo.toml --locked --all-targets -- -D warnings
./gradlew testDebugUnitTest lintDebug assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Rust tests exercise authentication success/failure, malformed requests, IPv4/IPv6
and hostname connections, TCP half-close and byte counters, sustained one-way
traffic, UDP payloads and source filtering, fragmented UDP rejection, UDP-over-TCP
framing, occupied ports, shutdown, and restart. `native/src/main.rs` is a host CLI
for integration testing with the same engine and JSON configuration.

The Android service is not exported. Start/stop it through the UI or notification.
On Android 17+, the first Start requests local-network access so other devices can
connect. Denying it leaves the proxy stopped with an App permissions shortcut.
Settings are editable while stopped. Credentials remain in app-private preferences;
Android backup is disabled. SOCKS5 username/password authentication does not encrypt
traffic; use a trusted network or an encrypted tunnel where required.

## Performance testing

The **Low-latency TCP** switch controls TCP_NODELAY. Measure with your own
clients and network before drawing performance conclusions.
Stop the proxy, change the switch, save, and restart to compare configurations.

Start a controlled endpoint on a computer reachable from the phone:

```bash
python3 scripts/benchmark.py serve --bind COMPUTER_LAN_IP --port 9000
python3 scripts/benchmark.py run --proxy PHONE_LAN_IP --target COMPUTER_LAN_IP --label rust --clients 1 > result.json
```

Disable authentication for this synthetic benchmark. Use the LAN address, not
`127.0.0.1`, for a remote endpoint. Repeat with `--clients 4`, alternate variants,
and keep power/screen state, network, worker count, and other traffic consistent.
Stop the endpoint when finished. JSON includes connection setup, warmed-up 32-byte
round trips (median/p95), and verified upload/download transfers. Throughput is
per client, not synchronized aggregate bandwidth. Host-loopback results do not
establish Android performance.

## Architecture

- `app/src/main/java/de/shyim/sockspring`: Compose UI, preferences, and foreground service.
- `native/src/lib.rs`: Tokio engine, SOCKS protocols, JNI lifecycle, atomic counters.
- JNI is used only to start/stop and read counters. Traffic never crosses into Kotlin.
- At most 512 concurrent sessions; bounded UDP queues and destination tables,
  handshake/connect/idle timeouts, and cancellation on shutdown.
- UDP association packets must match the TCP client's IP and selected UDP port.
  Responses are accepted only from recently contacted destinations. SOCKS UDP
  fragmentation is unsupported.
- Interface binding depends on OS permissions. Invalid or inaccessible bind
  addresses result in startup or connection errors; this app does not bypass
  Android network permissions or act as a system VPN.

