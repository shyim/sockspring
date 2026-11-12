# Sockspring privacy policy

Effective date: November 12, 2026

Sockspring (`de.shyim.sockspring`) is an open-source SOCKS5 proxy server for Android, published by **Shyim**. Contact: **socksspring@shyim.de**.

## What the app does

When you start the proxy, devices you configure to use it can send network requests through your Android device. Sockspring receives the client connection and forwards its traffic to the destination requested by that client. DNS lookups use the device's configured resolver. Destination services, DNS providers, and network operators handle this traffic under their own policies.

Shyim does not operate a proxy backend for this app. The app does not send traffic, browsing history, credentials, or usage statistics to Shyim. It has no advertising, analytics, crash-reporting SDK, or account system.

## Information handled on your device

The proxy processes client and destination IP addresses, domain names, ports, and network payloads in memory to route connections. It maintains temporary connection state and session traffic counters. It does not write a browsing-history or traffic-content log. Temporary connection state is discarded when connections end or the service stops; session counters reset when a new session starts.

Your server settings, including an optional proxy username and password, are saved in app-private Android preferences. These credentials control access to your proxy; they are not a Shyim account. Preferences are not encrypted by the app. Android cloud backup and device-transfer backup are disabled for this data.

Copying or sharing the proxy address is optional. Copy places it on the system clipboard; Share sends it to the app or recipient you choose.

## Permissions and background operation

Internet and local-network access allow the proxy to accept and forward connections. A foreground service keeps a proxy session available while you use other apps. A notification shows that the server is active and offers a Stop action. Notification permission allows that notification to appear normally. The optional Start after reboot setting restarts the proxy after a device reboot; it is off by default.

The app does not request access to your contacts, photos, microphone, camera, or precise location.

## Security and your choices

SOCKS5 itself does not encrypt traffic or proxy usernames and passwords. Applications using end-to-end encryption, such as HTTPS, retain that encryption through the proxy. Use trusted networks and enable proxy authentication when appropriate. Sockspring is not a VPN and does not automatically route all device traffic.

You can stop forwarding at any time using Stop in the app or notification. Disable Start after reboot to prevent automatic startup. Clear the app's storage in Android settings or uninstall it to delete saved preferences. Shyim has no server-side app account or stored proxy data to delete.

## Support and changes

If you email support, your email address and the information you choose to send are used to respond to your request. Do not include passwords or sensitive traffic. Support correspondence is separate from the app's network operation; you may request its deletion by email, subject to any applicable retention requirements.

Changes to this policy will be published in this file with an updated effective date. The source code and policy are available at https://github.com/shyim/sockspring.
