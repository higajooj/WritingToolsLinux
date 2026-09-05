# Run Writing Tools on Wayland

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

Install `wl-clipboard`, `xdg-desktop-portal`, and your compositor's portal
backend. For Hyprland, use `xdg-desktop-portal-hyprland`.

Writing Tools creates
`~/.local/share/applications/com.writingtools.WritingTools.desktop` on first
launch, respecting `XDG_DATA_HOME` if set. It points to the interpreter and
checkout used to launch the app. Existing entries are left unchanged. The
GlobalShortcuts portal requires this entry to bind shortcuts.

After relocating a checkout, update the existing entry's `Exec` and `Icon`
paths, or delete it and relaunch to have a fresh one written.

## Shortcuts

Shortcut registration uses the desktop portal. Wayland cannot read another
application's selection, so copy text before invoking Writing Tools. If the
compositor cannot paste the result, Writing Tools leaves it on the clipboard.

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

## Floating the popup

The shortcut popup is a frameless top-level window. Wayland clients cannot set
its position or request utility-window behavior. In a tiling compositor, add a
rule for the popup's app ID (`com.writingtools.WritingTools`) and title
(`Writing Tools`). These match only the popup, not the settings, about, or
response windows.

For Hyprland's `hyprland.conf`:

```ini
windowrule = float, class:^(com\.writingtools\.WritingTools)$, title:^(Writing Tools)$
windowrule = move cursor_x cursor_y+20, class:^(com\.writingtools\.WritingTools)$, title:^(Writing Tools)$
```

For Hyprland's Lua configuration:

```lua
hl.window_rule({
    name  = "writing-tools-popup",
    match = { class = [[^com\.writingtools\.WritingTools$]], title = [[^Writing Tools$]] },
    float = true,
    move  = "cursor_x cursor_y+20",
})
```

`cursor_x` and `cursor_y` are Hyprland's own cursor coordinates, so the popup
opens just below the pointer. Reload with `hyprctl reload`.

Other compositors need their own equivalent. For sway:

```
for_window [app_id="com.writingtools.WritingTools" title="Writing Tools"] floating enable
```

[Back to README](../README.md)
