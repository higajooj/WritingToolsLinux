# 👨‍💻 To Run Writing Tools Directly from the Source Code

If you prefer to run the program directly from the `main.py` file, follow these OS-specific instructions.

**1. Download the Code**
- Click the green `<> Code ▼` button toward the very top of this page, and click `Download ZIP`.

**2. Install Dependencies**  
After extracting the folder, open your **Terminal** (or **Command Prompt**) in the relevant directory.

- Windows:
   ```bash
   cd path\to\Windows_and_Linux
   pip install -r requirements.txt
   ```

- Linux:
   ```bash
   cd /path/to/Windows_and_Linux
   pip3 install -r requirements.txt
   # Wayland users should also install their compositor's portal backend
   # and wl-clipboard (for Hyprland: xdg-desktop-portal-hyprland).
   ```
   On Wayland, install the included `com.writingtools.WritingTools.desktop`
   entry. It identifies Writing Tools to the portal and provides its icon.
   Replace `/path/to/WritingTools` with your checkout path, then run:
   ```bash
   mkdir -p ~/.local/share/applications
   cp com.writingtools.WritingTools.desktop ~/.local/share/applications/
   update-desktop-database ~/.local/share/applications 2>/dev/null || true
   ```
Of course, you'll need to have [Python installed](https://www.python.org/downloads/)!

**3. Run the Program**
- **Windows:**
   ```bash
   python main.py
   # Tip: If you want Writing Tools to remain running even after you close your Terminal window, run `pythonw main.py` instead of `python main.py`.

   ```

- **Linux:**
   ```bash
   python3 main.py
   ```

**4. Configure Wayland shortcuts**

GNOME and KDE use the shortcut requested through the portal. wlroots
compositors require a binding in their own configuration. Writing Tools
registers `com.writingtools.WritingTools:global` and
`com.writingtools.WritingTools:button:<Name>` for each button hotkey. It logs
the required bindings at startup.

Add this to Hyprland's `hyprland.conf`:
```ini
bind = SUPER, P, global, com.writingtools.WritingTools:global
```
Reload Hyprland with `hyprctl reload`. Run `hyprctl globalshortcuts` while
Writing Tools is open to list the registered IDs. If it shows no entries, the
portal registration failed. Check the app log.


### [**◀️ Back to main page**](https://github.com/theJayTea/WritingTools)
