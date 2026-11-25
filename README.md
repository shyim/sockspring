<p align="center">
  <img src="fastlane/metadata/android/en-US/images/featureGraphic.png" alt="Sockspring — Your phone. Your proxy. SOCKS5 for Android." width="800">
</p>

<h1 align="center">Sockspring</h1>

<p align="center">
  A SOCKS5 proxy and WireGuard server for Android.<br>
  Built with Material 3 Expressive and a native Rust engine.
</p>

<p align="center">
  <a href="#getting-started">Getting started</a> ·
  <a href="docs/development.md">Build from source</a> ·
  <a href="PRIVACY.md">Privacy</a> ·
  <a href="https://github.com/shyim/sockspring/issues">Report an issue</a>
</p>

---

Sockspring turns your Android device into a proxy for your other devices. Start the server, copy its address, and connect a SOCKS5-compatible client, or scan a QR code with the WireGuard app to send a whole device through your phone. A clear dashboard keeps connections and traffic visible, with a Stop action always available in the app and its ongoing notification.

**Android 7.0+ · No ads · No analytics · No app account · MIT licensed**

## Why Sockspring?

Sockspring is my escape hatch when traveling. I want reliable internet for my laptop and other devices without hunting for Wi-Fi. An unlimited travel eSIM can cover my phone, while hotspot limits make sharing that connection harder.

I built Sockspring to give my other devices another way to use that connection: a SOCKS5 proxy and a WireGuard server running on my phone, without root. It is a small, practical tool for working around hotspot limitations with compatible clients. Your devices still need a network connection to the phone, and what works depends on the eSIM plan and carrier.

## A closer look

<p align="center">
  <img src="fastlane/metadata/android/en-US/images/phoneScreenshots/1-overview.png" alt="Light theme showing an active proxy and its connection address" width="240">
  &nbsp;
  <img src="fastlane/metadata/android/en-US/images/phoneScreenshots/2-traffic-dark.png" alt="Dark theme showing live upload, download, and connection statistics" width="240">
  &nbsp;
  <img src="fastlane/metadata/android/en-US/images/phoneScreenshots/3-settings.png" alt="Server settings with listening address, port, and optional authentication" width="240">
</p>

<p align="center"><sub>Actual app screenshots: overview, live traffic, and settings.</sub></p>

## Features

| Everyday controls | Flexible networking |
| --- | --- |
| One-tap start and stop | SOCKS5 TCP and UDP support |
| Copy or share your proxy address | IPv4, IPv6, and hostname resolution |
| Live traffic, connections, and session uptime | Optional username/password authentication |
| Light and dark themes | Configurable listening address and port |
| Ongoing notification with a Stop action | Low-latency TCP option using TCP_NODELAY |
| Optional startup after reboot | Advanced source binding, workers, and UDP-over-TCP support |
| QR codes for the WireGuard app | Built-in WireGuard server, no root or system VPN needed |

## Getting started

Sockspring is being prepared for Google Play and is **not yet listed there**. You can [build an APK from source](docs/development.md#build).

1. Open Sockspring on your Android device.
2. Tap **Start proxy** and allow the requested permissions. Android 17 and newer require local-network access.
3. On another device that can reach your phone, configure a SOCKS5-compatible client with the address shown in **Overview**. The default port is `1080`.
4. Use **Stop proxy** in the app or **Stop** in its notification when you are finished.

For example, from a computer on the same reachable network:

```sh
curl --socks5-hostname PHONE_ADDRESS:1080 https://example.com/
```

Replace `PHONE_ADDRESS` with the address shown in the app. Wi-Fi client isolation, hotspot settings, and network restrictions can prevent devices from reaching each other.

To change the port, enable authentication, or adjust advanced networking options, stop the proxy and open **Settings**.

### WireGuard

SOCKS5 only helps apps that have a proxy setting. The built-in WireGuard server carries all traffic of a device instead.

1. In **Settings**, turn on **WireGuard server** and save.
2. On the **Devices** tab, tap **Add device** and name it.
3. Scan the QR code with the [WireGuard app](https://www.wireguard.com/install/) on the other device, or copy the configuration into a `.conf` file.
4. Start the proxy and activate the tunnel on the other device.

The default port is UDP `51820`. Each device gets its own key and tunnel address, and a device needs a new QR code after you change the WireGuard port, endpoint host, or tunnel address. If the other device reaches your phone through a public address or port forward, enter that under **Endpoint host**.

Sockspring runs WireGuard entirely inside the app. It does not create a VPN on the phone and needs no root: traffic from your devices leaves through ordinary app connections, exactly like SOCKS5 traffic. TCP, UDP, ping, and DNS through the phone's resolvers work; fragmented IP packets and protocols other than TCP, UDP, and ICMP echo are not forwarded.

## Privacy and security

Settings stay in app-private storage, and Android backup is disabled. Sockspring has no advertising, analytics, or publisher-operated proxy backend. Network requests go to the destinations your clients choose. Read the full [privacy policy](PRIVACY.md) for details.

**SOCKS5 does not encrypt traffic or proxy passwords.** Use a trusted network and enable authentication when appropriate. HTTPS and other end-to-end encrypted connections retain their own encryption through the proxy.

WireGuard encrypts traffic between your device and the phone only; beyond the phone it travels as it would from the phone itself. A WireGuard configuration contains that device's private key, so share it only with the device it is for. Remove a device on the **Devices** tab to revoke its access.

Sockspring is not a VPN service. It does not provide remote servers, and it never routes the phone's own traffic anywhere.

## Development

The Android interface is written in Kotlin with Jetpack Compose. A Rust/Tokio engine handles proxy and WireGuard traffic; JNI is used only for lifecycle control and statistics.

- [Development guide](docs/development.md) — toolchain, builds, tests, architecture, and benchmarking.
- [Play release kit](distribution/play/README.md) — signing, store assets, and submission instructions.

Package name: `de.shyim.sockspring`.

Contributions are welcome. For a bug report, include the app version, Android version, reproduction steps, and expected behavior. Keep passwords and sensitive network data out of reports.

## Support

Open an [issue on GitHub](https://github.com/shyim/sockspring/issues) or email [socksspring@shyim.de](mailto:socksspring@shyim.de).

Published by **Shyim**.

## License

Released under the [MIT License](LICENSE).

"WireGuard" is a registered trademark of Jason A. Donenfeld. Sockspring is an independent implementation and is not affiliated with or endorsed by the WireGuard project.
