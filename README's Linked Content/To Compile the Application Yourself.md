# Build Writing Tools for Linux

Run from the repository root:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
python pyinstaller-build-script.py
```

The build script uses PyInstaller to create `dist/Writing Tools/`, containing
the executable, libraries, icons, translations, backgrounds, and default options.
Keep the entire directory together. Each build replaces the previous build output.

Launch it with:

```sh
"./dist/Writing Tools/Writing Tools"
```

Use a writable directory: settings and custom options are saved beside the
executable. Your source checkout's private `config.json` is never bundled.
Builds include the checkout's current `options.json`; review custom prompts
before sharing a distribution.

Build on Linux for Linux. The target system must provide a compatible Linux
runtime and desktop session; Wayland also needs the portal backend and clipboard
utilities described in the [source instructions](To%20Run%20Writing%20Tools%20Directly%20from%20the%20Source%20Code.md).

To use a desktop launcher, set its `Exec` to the built executable (quote paths
containing spaces) and `Icon` to the distribution's `icons/app_icon.png`.
Keep `StartupWMClass=com.writingtools.WritingTools`.

[Back to README](../README.md)
