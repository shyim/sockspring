# Sockspring Play release kit

Package: `de.shyim.sockspring`. Store title: **Sockspring — SOCKS5 Proxy**.
Publisher: **Shyim**. Support: **socksspring@shyim.de** (spelling provided by publisher).
Version: **1.0.0**, code **1**. Default language: **en-US**. Category: **Tools**.

## Files

- `../../fastlane/metadata/android/en-US/`: title, short/full description, support details, privacy URL, release notes and images in Fastlane Supply layout.
- `listing.json`: Console settings. Pricing, audience and country selection are recommendations for publisher review.
- `declarations.md`: app access, permissions, foreground-service submission copy, Data safety assessment and content-rating guidance.
- `../../PRIVACY.md`: public policy; also bundled in the app under Settings → Privacy policy.
- `../../artwork/`: editable store artwork sources and generation instructions.

## Build and signing

Set `JAVA_HOME` and `ANDROID_HOME`, install the dependencies in the root README, then:

```sh
python3 scripts/create-upload-key.py  # once only; refuses to replace a key
./gradlew bundleRelease assembleRelease lintRelease testDebugUnitTest
```

Outputs: `app/build/outputs/bundle/release/app-release.aab` and
`app/build/outputs/apk/release/app-release.apk`. Upload the **AAB** to Play.
When `upload-signing.properties` is absent, release outputs are **unsigned**.
Debug builds use Android's local debug key.

**Back up `.signing/sockspring-upload.p12` and `upload-signing.properties` together in secure storage.** Both are ignored by Git. The password is in the properties file; do not paste it into issues or build logs. `.signing/upload-certificate.pem` is the public upload certificate.
Enable Play App Signing and let Google generate the app signing key. The local key is the upload key. The Play-installed app will then use Google's signing certificate, so a locally installed release APK may need uninstalling before installing from Play. Increment `versionCode` for every subsequent upload.

CI deliberately builds unsigned artifacts without signing secrets. Do not upload its unsigned AAB directly. CI artifacts are labeled unsigned; the workflow does not automatically publish releases.

## Submission sequence

1. Source is published at `https://github.com/shyim/sockspring`. Verify `https://github.com/shyim/sockspring/blob/main/PRIVACY.md` is accessible without login before entering it in Console. The URL refers to the rendered Markdown page, not GitHub's edit page or a download.
2. Create a Play Console app with the title/default language above, app type App, and chosen pricing.
3. Complete developer-account verification, contact details, country distribution, audience and content rating in your account. Account-specific testing requirements are shown by Console; satisfy them before production access.
4. Enter the supplied store text, contact email, website and privacy URL. Upload the icon, feature graphic and phone screenshots. Do not opt into TV distribution until TV navigation and TV-specific assets have been validated.
5. Complete App content using `declarations.md`. Review the proxy-specific Data safety caveat; do not automatically claim that all traffic is encrypted or that no data ever leaves the device.
6. Upload the signed AAB to internal testing first. Enable Play App Signing. Test the Play-delivered build on a device, including permissions, start/stop, TCP/UDP and background operation.
7. Use the public foreground-service demonstration video URL in `declarations.md` (or host it on your preferred video service), verify reviewer access, and submit the declaration. Play approval is not guaranteed by using `specialUse`.
8. Review the pre-launch report and Console policy checks, then submit for production review when eligible.

## Official references (checked November 12, 2026)

- [Listing assets and dimensions](https://support.google.com/googleplay/android-developer/answer/9866151)
- [Data safety definitions](https://support.google.com/googleplay/android-developer/answer/10787469)
- [Foreground-service declarations and video](https://support.google.com/googleplay/android-developer/answer/13392821)
- [Privacy policy / User Data requirements](https://support.google.com/googleplay/android-developer/answer/10144311)

The name is a proposed product name, not a trademark-clearance opinion. Store availability and final declarations must be checked during submission.
