# Play Console declaration drafts

These answers describe the current source and bundled dependencies. Review them against the final uploaded build and any later SDKs. They are not a claim of Play approval.

## App access — all functionality available without special access

No developer account, subscription, invitation, or login is required. Proxy authentication is optional and is configured locally by the user, not supplied by the publisher.

Reviewer instructions:
1. Open Sockspring and tap Start proxy. Allow notifications and, on Android 17+, local-network access when prompted.
2. Connect another device to a network that can reach the Android device. Configure a SOCKS5-compatible client with the address and port shown in Overview (default port 1080). No username/password is required by default.
3. Send a request through that client. Overview shows connections and transferred data.
4. Stop the server through the app or ongoing notification. Settings can be changed while stopped.

Example on a computer on the same reachable network:
`curl --socks5-hostname PHONE_ADDRESS:1080 https://example.com/`
Replace PHONE_ADDRESS with the app's displayed address. Client isolation or carrier/hotspot restrictions can prevent connections; use a permitted LAN.

## Ads, accounts, payments and category

- Contains ads: **No**. No advertising SDK or advertising ID permission.
- App accounts: **No account creation**. Optional proxy credentials are local access control.
- In-app purchases/subscriptions: **None**.
- App category: **Tools**. Suggested tags, if Console offers them: Network, Tools.
- Government, news, health, financial services: **No**.
- Target audience recommendation: **18 and over**; this is a networking utility, not designed or marketed for children. Publisher must select the intended audience.
- Content rating: complete IARC honestly; do not manually claim an age rating before IARC assigns it. The app contains no authored violence, sexual content, gambling, social feed, chat, or user-uploaded public content. It forwards arbitrary client-requested Internet traffic, so review any questionnaire question about Internet access rather than treating it as a content filter/browser or claiming restricted destinations.

## Foreground service: FOREGROUND_SERVICE_SPECIAL_USE

Use case: User-controlled SOCKS5 proxy server.

Functionality description:
Sockspring hosts a SOCKS5 server on the user's Android device. The user starts it in the app and configures their other devices to connect. The foreground service accepts connections and relays TCP and UDP traffic while the app is in the background. A persistent notification identifies the active proxy and provides Stop. The app also provides Stop on its dashboard. Optional startup after reboot is explicitly enabled by the user and disabled by default.

Impact if deferred:
The proxy must be available immediately after the user starts it. Delaying startup causes the user's configured client connections to fail or time out.

Impact if interrupted:
Interrupting the service closes active proxy connections and prevents new connections, disrupting requests from devices the user configured. The service runs only while the user has enabled the proxy and can be stopped at any time. This is a continuing server session, not a deferrable file-transfer job.

Prepared demonstration video: https://github.com/shyim/sockspring/blob/main/distribution/play/foreground-service-demo.mp4

The included recording shows Start, live traffic counters, and Stop on the real device. For a more detailed reviewer walkthrough, use this script:
- Show the app name and stopped Overview.
- Tap Start proxy and show the active state/address.
- Send a request through the proxy from another device; show traffic counters.
- Go to the home screen and open the notification shade; show the Sockspring notification and Stop action.
- Tap Stop; return to Overview and show the stopped state.
- Optionally show Start after reboot is off by default in Settings.

Do not submit a placeholder video URL. Any local recording must be uploaded to a location the reviewer can access.

## Other permissions

- INTERNET: accept and forward user-requested network traffic.
- ACCESS_LOCAL_NETWORK: Android 17 runtime permission for clients on the local network; requested on Start, denial handled without starting.
- POST_NOTIFICATIONS: display proxy status and a Stop action.
- RECEIVE_BOOT_COMPLETED: optional user-enabled startup after reboot; off by default.
- FOREGROUND_SERVICE / SPECIAL_USE: keep the user-enabled proxy session available.
- No VPNService, location, camera, microphone, contacts, storage, accessibility, package-query or advertising ID access. Do not declare this as a VpnService app merely because it is a proxy.

## Data safety assessment — requires final publisher review

**Do not equate “no developer backend” with “no off-device transmission.”** Google's definition covers data transmitted to third parties, and ephemeral handling can still require a form response. SOCKS5 can carry unencrypted traffic, so do not select “all collected data encrypted in transit.”

| Data / operation | Current behavior | Proposed treatment |
| --- | --- | --- |
| Saved settings and optional proxy credentials | App-private preferences; no cloud backup; never sent to Shyim | On-device processing, not declared as collection solely because it is stored locally |
| Session counters | In-memory counts; no analytics endpoint | On-device processing |
| Client IPs, requested destination names/addresses and payloads | Used to forward client requests and perform DNS; no traffic-history log or developer server | Review as off-device collection under the broad Play definition; destination/domain requests can fall under Web browsing history. Purpose: app functionality; required while proxying; local processing is ephemeral |
| Third-party destination transfer | Only to destinations requested by configured clients | Proposed user-initiated transfer exception to “sharing”; document the user-directed proxy purpose. This exception does not automatically exempt collection |
| End-to-end encrypted client payload | Forwarded without decryption or keys | E2E exception only where actually encrypted end to end; not a blanket exemption for all proxy traffic or DNS |
| Arbitrary unencrypted client payload | May contain data chosen by the client, which Sockspring does not classify | Resolve the applicable data-type declarations for this general-purpose proxy during Console review; do not blindly check every category or silently claim no data |
| Support email | User voluntarily contacts publisher outside the app | Described in privacy policy; app does not collect or send email itself |

Recommended starting point: disclose the forwarding behavior rather than pre-filling a “no collection” CSV. Review **Web browsing history** for requested destinations, with purpose **App functionality**, and whether **ephemeral processing** applies to the submitted data flow. The app's own memory retention is ephemeral; arbitrary destination providers may retain requests, so their behavior cannot be certified as ephemeral by this publisher. If Console guidance does not clearly classify generic pass-through traffic, obtain clarification from Play support before production submission.

No sale of data, advertising, personalization, fraud profiling, or developer analytics is implemented. There is no server-side account to delete. Users delete local settings through Android Clear storage or uninstall. Privacy policy: https://github.com/shyim/sockspring/blob/main/PRIVACY.md.
