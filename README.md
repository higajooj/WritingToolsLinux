# Writing Tools for Linux

Writing Tools is a Wayland-only AI writing assistant for Linux, built with
Python and PySide6. Copy text, then use a shortcut to proofread, rewrite,
change tone, summarize, or apply custom instructions. It supports Gemini,
ChatGPT subscriptions, Ollama, and OpenAI-compatible servers.

Run it from a checkout with `python main.py`. There is no X11, Windows, or
packaged-build support. The code is in `src/`, assets are in `assets/`, and
tests are in `tests/`.

## Run

Install `wl-clipboard`, `xdg-desktop-portal`, and your compositor's portal
backend. For Hyprland, use `xdg-desktop-portal-hyprland`.

From the repository root, create a virtual environment and install dependencies:

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
python main.py
```

With the virtual environment active, you can also launch the root `main.py`
by absolute path from any directory.

Configure your provider in the initial setup or Settings. The default shortcut
is `ctrl+space`; change it if it conflicts with another application. See
[Shortcuts](#shortcuts) for compositor bindings and
[Popup window rules](#popup-window-rules) for tiling desktop setup.

## Desktop integration

Writing Tools creates
`~/.local/share/applications/com.writingtools.WritingTools.desktop` on first
launch, using `$XDG_DATA_HOME/applications` instead if `XDG_DATA_HOME` is an
absolute path. It points to the interpreter and checkout used to launch the
app. Existing entries are left unchanged. The GlobalShortcuts portal requires
this entry to bind shortcuts.

After relocating a checkout, update the existing entry's `Exec` and `Icon`
paths, or delete it and relaunch to have a fresh one written.

## Shortcuts

Shortcut registration uses the desktop portal and requires a backend that
supports GlobalShortcuts. Copy text before invoking Writing Tools: the app
cannot read another application's selection on Wayland. If automatic pasting
is unavailable, the result stays on the clipboard for you to paste manually.

On desktops that handle shortcut assignment through the portal, configure the
shortcut when prompted. Hyprland requires a binding in its own configuration.
Writing Tools registers `com.writingtools.WritingTools:global` and
`com.writingtools.WritingTools:button:<Name>` for each button hotkey and logs
the required bindings at startup.

For Hyprland's `hyprland.conf`, bind Super+P with:

```ini
bind = SUPER, P, global, com.writingtools.WritingTools:global
```

Reload with `hyprctl reload`. Run `hyprctl globalshortcuts` while Writing Tools
is open to inspect registered IDs. If none appear, check the application log
for portal registration failures. See Hyprland's
[global shortcut documentation](https://wiki.hypr.land/configuring/core/binds/globals/)
for Lua configuration and further details.

## Popup window rules

The shortcut popup is a frameless top-level window whose placement is managed
by the compositor on Wayland. In a tiling compositor, match both its app ID
(`com.writingtools.WritingTools`) and title (`Writing Tools`) to float the popup
without affecting the settings, about, or response windows.

For Hyprland's Lua configuration (`hyprland.lua`):

```lua
hl.window_rule({
    name  = "writing-tools-popup",
    match = { class = [[^com\.writingtools\.WritingTools$]], title = [[^Writing Tools$]] },
    float = true,
    move  = { "cursor_x", "cursor_y+20" },
})
```

For `hyprland.conf` using the Hyprland 0.54 window-rule syntax:

```ini
windowrule {
    name = writing-tools-popup
    match:class = ^com\.writingtools\.WritingTools$
    match:title = ^Writing Tools$
    float = on
    move = cursor_x (cursor_y+20)
}
```

These rules place the popup just below the pointer using Hyprland's cursor
coordinates. Reload with `hyprctl reload`. See the official window-rule
documentation for [Lua](https://wiki.hypr.land/Configuring/Basics/Window-Rules/)
or [Hyprland 0.54](https://wiki.hypr.land/0.54.0/Configuring/Window-Rules/).

For sway, add this to your configuration to float the popup:

```ini
for_window [app_id="^com\.writingtools\.WritingTools$" title="^Writing Tools$"] floating enable
```

See sway's [configuration reference](https://github.com/swaywm/sway/blob/master/sway/sway.5.scd)
for window criteria and rules. Other compositors need their own equivalent.

## Features and configuration

- Proofread, rewrite, adjust tone, or apply a custom change to copied text.
- Summaries, key points, and tables in a response window with Markdown rendering.
- Custom buttons and shortcuts in `options.json`.
- Light/dark appearance and gradient/plain themes.
- Local models through Ollama or an OpenAI-compatible server.
- Browser-based ChatGPT subscription sign-in through the official Codex CLI.

Settings, including provider credentials, are stored locally in the ignored
`config.json` beside `main.py`. Keep it private. Custom buttons and their
shortcuts are stored in the root `options.json`. Text is sent to the provider
you configure when you invoke writing actions; local providers can keep
processing on your machine.

For Ollama, start the server, download a model, and select the Ollama provider
in Settings with that model's name. Alternatively, use the OpenAI-compatible
provider with base URL `http://localhost:11434/v1`.

### ChatGPT subscription access

To use models included with a ChatGPT plan, first install the official Codex
CLI:

```sh
npm install -g @openai/codex
```

Then choose **OpenAI Subscription (ChatGPT)** in Settings and select **Sign in
with ChatGPT**. Writing Tools opens the browser for authentication and loads
the models available to that account. The default **Automatic** model follows
the Codex default; you can also choose a model from the account dynamically.

Writing Tools uses its own Codex data directory, so signing in or out here does
not change the account used by your normal Codex CLI sessions. Authentication
is managed by Codex and is not stored in `config.json`. See OpenAI's official
[Codex CLI documentation](https://learn.chatgpt.com/docs/codex/cli) for current
installation and subscription details.

### OpenAI processing speed

Choose **Standard** or **Fast (priority)** in the **Speed** selector, independently
of the model. Standard is the default, including for existing configurations.
The choice is saved separately for each provider and applies to writing actions
and follow-up chat.

- **OpenAI Subscription (ChatGPT):** Speed applies to the selected model or
  **Automatic**. Fast uses more ChatGPT credits. Codex reports the speeds each
  model supports, so the selector greys out on a model that offers only
  Standard; **Automatic** always offers both. Every request states its speed
  explicitly. The override was verified against Codex CLI **0.153.4**; update
  Codex if using an older version.
- **OpenAI Compatible (For Experts):** Speed is available when the API Base URL
  is `https://api.openai.com/v1` (a trailing slash is accepted); the selector
  greys out for any other server, which receives no speed parameter. Fast costs
  more. Standard sends no speed parameter either, so the tier configured in your
  OpenAI Project still applies. Servers without authentication can still use an
  empty API key.

Fast availability also depends on the account, and requesting it does not
guarantee priority processing. Requests use the `priority` service tier, which
OpenAI supports for [Fast mode](https://developers.openai.com/api/docs/guides/fast-mode).
See also [Codex speed and credit usage](https://learn.chatgpt.com/docs/agent-configuration/speed).


## Development

Run the tests from the repository root:

```sh
QT_QPA_PLATFORM=offscreen python -m unittest discover -s tests
```

Update with `git pull`, then reinstall dependencies if `requirements.txt` changed.
Preserve `config.json` and any customizations to `options.json` when updating.

## Credits

Based on [Writing Tools by Jesai](https://github.com/theJayTea/WritingTools).
Contributions to this Linux fork belong at
[WritingToolsLinux](https://github.com/higajooj/WritingToolsLinux).

**1. [momokrono](https://github.com/momokrono):**

Added Linux support, switched to the pynput API to improve Windows stability. Added Ollama API support, the core logic for customisable buttons, and localization. Fixed misc. bugs and added graceful termination support by handling SIGINT signal.

@momokrono has been incredibly kind and helpful, and I'm forever grateful to have him as a contributor. Not only has he provided extensive help with code, but he's also played a big role in managing GitHub issues. - Jesai

**2. [Cameron Redmore (CameronRedmore)](https://github.com/CameronRedmore):**

Extensively refactored Writing Tools and added OpenAI Compatible API support, streamed responses, and the chat mode when no text is selected.

**3. [Soszust40 (Soszust40)](https://github.com/Soszust40):**

Helped add dark mode, the plain theme, tray menu fixes, and UI improvements.

**4. [Alok Saboo (arsaboo)](https://github.com/arsaboo):**

Helped improve the reliability of text selection.

**5. [raghavdhingra24](https://github.com/raghavdhingra24):**

Made the rounded corners anti-aliased & prettier.

**6. [ErrorCatDev](https://github.com/ErrorCatDev):**

Significantly improved the About window, making it scrollable and cleaning things up. Also improved our .gitignore & requirements.txt.

**7. [Vadim Karpenko](https://github.com/Vadim-Karpenko):**

Helped add the start-on-boot setting!

### Historical contributors
The retired native port was created by **[Arya Mirsepasi](https://github.com/Aryamirsepasi)**.

Over so many emails, @Aryamirsepasi has been someone I truly look up to, and it's rare to find people as kind as him. We're incredibly grateful for all his contributions here! — Jesai

**1. [Joaov41](https://github.com/Joaov41):**

Developed the amazing picture processing functionality in Gemini for WritingTools, allowing the app to now work with images in addition to text!

**2. [drankush](https://github.com/drankush):**

Fixed an issue that caused the app to fail in completing requests when the OpenAI provider was configured with a custom Base URL (e.g., for Groq or other compatible services).

**3. [gdmka](https://github.com/gdmka):**

- Added the change that makes the ResponseView remember the user’s preferred text size across app launches. 
- Implemented ability to set custom provider per each command. 



## License

Distributed under the GNU General Public License v3.0. See [LICENSE](LICENSE).
