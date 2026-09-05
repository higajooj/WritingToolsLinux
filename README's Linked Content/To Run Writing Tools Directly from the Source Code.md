# Run Writing Tools on Linux

From the repository root, create a virtual environment and install dependencies:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
python main.py
```

Settings are saved in the ignored `config.json` beside `main.py`.
Custom buttons are stored in the root `options.json`. Keep these files when updating.
Application code lives in `src/`; icons, backgrounds, and translations live in
`assets/`. You can launch the root `main.py` by absolute path from any directory.

## Desktop integration

On Wayland, install `wl-clipboard`, `xdg-desktop-portal`, and the portal backend
for your desktop (for Hyprland: `xdg-desktop-portal-hyprland`).

Writing Tools creates
`~/.local/share/applications/com.writingtools.WritingTools.desktop` on first
launch, respecting `XDG_DATA_HOME` if set. The entry uses the interpreter and
checkout that launched it. Existing entries stay unchanged.

After relocating a checkout, update the existing entry's `Exec` and `Icon`
paths. After updating from the old layout, change the source icon path to
`assets/icons/app_icon.png`; the launch command stays the same. To install the bundled entry manually, replace its
`/path/to/WritingTools` placeholders with your checkout's absolute path, then run:

```sh
mkdir -p ~/.local/share/applications
cp com.writingtools.WritingTools.desktop ~/.local/share/applications/
update-desktop-database ~/.local/share/applications
```

## Shortcuts

On X11, Writing Tools uses pynput for global shortcuts, selection capture,
and pasting. Change the shortcut in Settings if another application uses it.

On Wayland, shortcut registration uses the desktop portal. Automatic selection
capture and pasting depend on compositor support; use the clipboard when these
are unavailable.

GNOME and KDE use the shortcut requested through the portal. wlroots
compositors require a binding in their own configuration. Writing Tools
registers `com.writingtools.WritingTools:global` and
`com.writingtools.WritingTools:button:<Name>` for each button hotkey and logs
the required bindings at startup.

For Hyprland, add this to `hyprland.conf`:

```ini
bind = SUPER, P, global, com.writingtools.WritingTools:global
```

Reload with `hyprctl reload`. Run `hyprctl globalshortcuts` while Writing Tools
is open to inspect registered IDs. If none appear, check the application log
for portal registration failures.

[Back to README](../README.md)
