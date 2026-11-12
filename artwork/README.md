# Store artwork

`generate.py` is the editable source for the store icon and feature graphic. It uses the same four-node geometry and lime/ink colors as the Android launcher icon. It draws original graphics, not a screenshot mockup.

```sh
python3 -m venv /tmp/sockspring-artwork
/tmp/sockspring-artwork/bin/pip install Pillow
/tmp/sockspring-artwork/bin/python artwork/generate.py --font /path/to/Arial.ttf
```

Output goes to `fastlane/metadata/android/en-US/images/`. The icon is 512×512 RGBA, with a full square background so Google Play can apply its own mask. The feature graphic is 1024×500 RGB.

`phoneScreenshots` contains real device captures of the renamed app. No credentials, personal notifications, or manufactured performance claims belong in these screenshots. Capture with a 1080×1920 viewport for the store, then restore the device's original display settings.
