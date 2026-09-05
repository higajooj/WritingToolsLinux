# Writing Tools for Linux

Writing Tools is a Wayland-only AI writing assistant for Linux, built with
Python and PySide6. Copy text, then use a shortcut to proofread, rewrite,
change tone, summarize, or apply custom instructions. It supports Gemini,
Ollama, and OpenAI-compatible servers.

Run it from a checkout with `python main.py`. There is no X11, Windows, or
packaged-build support. The code is in `src/`, assets are in `assets/`, and
tests are in `tests/`.

## Run

```sh
python3 -m venv .venv
source .venv/bin/activate
python -m pip install -r requirements.txt
python main.py
```

Install `wl-clipboard`, `xdg-desktop-portal`, and your compositor's portal
backend. For Hyprland, use `xdg-desktop-portal-hyprland`.

Configure your provider in the initial setup or Settings. The default shortcut
is `ctrl+space`; change it if it conflicts with another application. wlroots
compositors also need a shortcut binding in their configuration. See the
[source and desktop setup guide](README's%20Linked%20Content/To%20Run%20Writing%20Tools%20Directly%20from%20the%20Source%20Code.md)
for shortcut bindings and popup window rules.

## Features and configuration

- Proofread, rewrite, adjust tone, or apply a custom change to copied text.
- Summaries, key points, and tables in a response window with Markdown rendering.
- Custom buttons and shortcuts in `options.json`.
- Light/dark appearance and gradient/plain themes.
- Local models through Ollama or an OpenAI-compatible server.

Settings, including provider credentials, are stored locally in the ignored
`config.json`. Keep it private. Text is sent to the provider you configure
when you invoke writing actions; local providers can keep processing on your
machine.

For Ollama, start the server, download a model, and select the Ollama provider
in Settings with that model's name. Alternatively, use the OpenAI-compatible
provider with base URL `http://localhost:11434/v1`.

## Using claude-code-proxy

Writing Tools can connect to a local `claude-code-proxy` through the **OpenAI Compatible (For Experts)** provider. Sign in to your upstream provider through the proxy first, then start (or restart) the proxy with its OpenAI-compatible routes enabled:

```sh
CCP_CODEX_RESPONSES_API=1 claude-code-proxy serve
```

In Writing Tools' AI setup or Settings, enter:

| Setting | Value |
| --- | --- |
| API Base URL | `http://127.0.0.1:18765/v1` |
| API Key | Leave blank |
| API Model | `gpt-5.6-sol`, or another ID listed by `claude-code-proxy models` |
| API Organisation / API Project | Leave blank |

An empty API key sends requests without an Authorization header. The proxy handles upstream authentication. Other OpenAI-compatible servers that require a key still use the key entered in Settings.

If requests return HTTP 404, check that the proxy was started with `CCP_CODEX_RESPONSES_API=1` and that the base URL ends in `/v1`, without `/chat/completions`. Older Writing Tools versions that require a key can use `unused` as a placeholder; the proxy ignores incoming bearer credentials.

Known proxy limitation: with claude-code-proxy 0.1.35 and the Codex backend, follow-up chat can fail with `Invalid value: 'input_text'`. The proxy translates assistant history into the wrong upstream content type. Single-request writing actions work; follow-up chat requires a fix in the proxy.


## Development

Run the tests from the repository root:

```sh
QT_QPA_PLATFORM=offscreen python -m unittest discover -s tests
```

Update with `git pull`, then reinstall dependencies if `requirements.txt` changed.

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
