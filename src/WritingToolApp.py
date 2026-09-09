import gettext
import json
import logging
import os
import signal
import sys
import threading
import time

import darkdetect
import ui.AboutWindow
import ui.CustomPopupWindow
import ui.OnboardingWindow
import ui.ResponseWindow
import ui.SettingsWindow
from aiprovider import (
    GeminiProvider,
    OllamaProvider,
    OpenAICompatibleProvider,
    OpenAISubscriptionProvider,
    obfuscate_api_key,
)
from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtCore import QLocale, Signal, Slot
from PySide6.QtWidgets import QApplication, QMessageBox
from app_paths import app_root, asset_root
from platform_input import APP_ID, WaylandInputBackend, validate_trigger

_ = gettext.gettext


class _SelectedTextHolder:
    """
    Carries the result of an async clipboard capture from `_show_popup` to
    `process_option_thread`. The capture thread sets `text` and signals
    `ready` once done.
    """
    __slots__ = ("text", "ready")

    def __init__(self):
        self.text = ""
        self.ready = threading.Event()


class WritingToolApp(QtWidgets.QApplication):
    """
    The main application class for Writing Tools.
    """
    output_ready_signal = Signal(str)
    show_message_signal = Signal(str, str)  # a signal for showing message boxes
    hotkey_triggered_signal = Signal()
    followup_response_signal = Signal(str)


    def __init__(self, argv):
        super().__init__(argv)
        self.setApplicationName("Writing Tools")
        self.setDesktopFileName(APP_ID)
        self.current_response_window = None
        logging.debug('Initializing WritingToolApp')
        self.output_ready_signal.connect(self.handle_output_ready)
        self.show_message_signal.connect(self.show_message_box)
        self.hotkey_triggered_signal.connect(self.on_hotkey_pressed)
        self.config = None
        self.config_path = None
        self.load_config()

        # Run any pending config migrations in a single pass (single restart).
        self._migrate_config()

        self.options = None
        self.options_path = None
        self.load_options()
        self.onboarding_window = None
        self.popup_window = None
        self.tray_icon = None
        self.tray_menu = None
        self.settings_window = None
        self.about_window = None
        self.registered_hotkey = None
        self.input_backend = WaylandInputBackend(self)
        self.paused = False
        self.toggle_action = None

        # Serialize clipboard reads from rapid shortcut presses.
        self.current_text_holder = None
        self._capture_lock = threading.Lock()

        self._ = gettext.gettext

        # Initialize the ctrl+c hotkey listener
        self.ctrl_c_timer = None
        self.setup_ctrl_c_listener()

        # Setup available AI providers
        self.providers = [
            GeminiProvider(self),
            OpenAISubscriptionProvider(self),
            OpenAICompatibleProvider(self),
            OllamaProvider(self),
        ]

        if not self.config:
            logging.debug('No config found, showing onboarding')
            self.show_onboarding()
        else:
            logging.debug('Config found, setting up hotkey and tray icon')

            # Initialize the current provider, defaulting to Gemini
            provider_name = self.config.get('provider', 'Gemini')

            self.current_provider = next((provider for provider in self.providers if provider.provider_name == provider_name), None)
            if not self.current_provider:
                logging.warning(f'Provider {provider_name} not found. Using default provider.')
                self.current_provider = self.providers[0]

            self.current_provider.load_config(self.config.get("providers", {}).get(provider_name, {}))

            self.create_tray_icon()
            self.register_hotkey()

            try:
                lang = self.config['locale']
            except KeyError:
                lang = None
            self.change_language(lang)

        self.recent_triggers = []  # Track recent hotkey triggers
        self.TRIGGER_WINDOW = 1.5  # Time window in seconds
        self.MAX_TRIGGERS = 3  # Max allowed triggers in window

    def setup_translations(self, lang=None):
        if not lang:
            lang = QLocale.system().name().split('_')[0]

        try:
            translation = gettext.translation(
                'messages',
                localedir=os.path.join(asset_root(), 'locales'),
                languages=[lang]
            )
        except FileNotFoundError:
            translation = gettext.NullTranslations()

        translation.install()
        # Update the translation function for all UI components.
        self._ = translation.gettext
        ui.AboutWindow._ = self._
        ui.SettingsWindow._ = self._
        ui.ResponseWindow._ = self._
        ui.OnboardingWindow._ = self._
        ui.CustomPopupWindow._ = self._

    def retranslate_ui(self):
        self.update_tray_menu()

    def change_language(self, lang):
        self.setup_translations(lang)
        self.retranslate_ui()

        # Update all other windows
        for widget in QApplication.topLevelWidgets():
            if widget != self and hasattr(widget, 'retranslate_ui'):
                widget.retranslate_ui()

    def check_trigger_spam(self):
        """
        Check if hotkey is being triggered too frequently (3+ times in 1.5 seconds).
        Returns True if spam is detected.
        """
        current_time = time.time()
        
        # Add current trigger
        self.recent_triggers.append(current_time)
        
        # Remove old triggers outside the window
        self.recent_triggers = [t for t in self.recent_triggers 
                            if current_time - t <= self.TRIGGER_WINDOW]
        
        # Check if we have too many triggers in the window
        return len(self.recent_triggers) >= self.MAX_TRIGGERS

    def load_config(self):
        """
        Load the configuration file.
        """
        self.config_path = os.path.join(app_root(), 'config.json')
        logging.debug(f'Loading config from {self.config_path}')
        if os.path.exists(self.config_path):
            with open(self.config_path, 'r') as f:
                self.config = json.load(f)
                logging.debug('Config loaded successfully')
        else:
            logging.debug('Config file not found')
            self.config = None

    def _migrate_config(self):
        """
        One-shot config migration. Catches any user up to the current schema
        (v9) regardless of where they started — v7, v8, or already current —
        in a single pass with at most one restart.

        Each version is gated on its own `is_config_file_updated_for_v{N}`
        flag, so re-running this is a no-op once everything's caught up.
        Bump CURRENT_CONFIG_VERSION and add a `# v{N}` block when adding a
        new migration step.

        v8 (introduced 2025):
          • Google removed Gemini 2.0 from the free API → bump model.
          • Obfuscate plaintext Gemini API keys (defeats Ctrl+F scanning).
          The custom-model input field didn't exist pre-v8, so any pre-v8
          model value is safe to overwrite — there's nothing custom to
          preserve.

        v9 (introduced 2026):
          • Google deprecated the Gemma 3 family. Migrate every v8 user on a
            now-deprecated preset to the new default (`gemini-flash-latest`)
            so they immediately land on the fast experience. They can opt
            into the unlimited-but-slow Gemma 4 options from the dropdown
            if they hit the 20/day Flash quota.
            Preserve custom model values (which DID exist by then) so a user
            who picked, say, `gemini-3.1-pro-preview` doesn't get reset.
          • SDK migration to `google-genai` is code-side only; nothing to
            do in config.
        """
        CURRENT_CONFIG_VERSION = 9
        # Default for new installs and migrating users.
        NEW_DEFAULT_MODEL = 'gemini-flash-latest'
        # v8 -> v9 model mapping. Every retired preset is bumped to the new
        # default so users get the fast Flash-tier experience by default.
        V8_TO_V9_MAP = {
            'gemma-3-27b-it':           NEW_DEFAULT_MODEL,
            'gemma-3-4b-it':            NEW_DEFAULT_MODEL,
            'gemini-flash-lite-latest': NEW_DEFAULT_MODEL,
            # 'gemini-flash-latest' itself is already current — no entry needed.
        }

        # New user (no config yet) — onboarding will create a fresh, current
        # config; nothing to migrate.
        if not self.config:
            logging.debug('No config to migrate (new user)')
            return

        needs_v8 = not self.config.get('is_config_file_updated_for_v8', False)
        needs_v9 = not self.config.get('is_config_file_updated_for_v9', False)

        if not needs_v8 and not needs_v9:
            logging.debug('Config already up-to-date, no migration needed')
            return

        logging.info(f'Running config migration (needs_v8={needs_v8}, needs_v9={needs_v9})...')

        config_changed = False
        gemini_config = (
            self.config.get('providers', {}).get('Gemini (Recommended)')
        )

        if gemini_config is not None:
            old_model = gemini_config.get('model_name', '')

            # v8: pre-v8 users didn't have a custom-model field, so we can
            # bump unconditionally. We skip the historical "v8 default of
            # gemma-3-27b-it" intermediate stop and jump straight to the
            # current default.
            if needs_v8:
                if old_model != NEW_DEFAULT_MODEL:
                    gemini_config['model_name'] = NEW_DEFAULT_MODEL
                    logging.info(f'[v8] Bumped Gemini model "{old_model}" -> "{NEW_DEFAULT_MODEL}"')
                    config_changed = True

                # Obfuscate the API key. The helper is idempotent — already-
                # obfuscated keys (with the `enc:` prefix) pass through
                # unchanged.
                api_key = gemini_config.get('api_key', '')
                if api_key:
                    new_key = obfuscate_api_key(api_key)
                    if new_key != api_key:
                        gemini_config['api_key'] = new_key
                        logging.info('[v8] Obfuscated plaintext Gemini API key')
                        config_changed = True

            # v9: only runs for users coming from v8. Preserve tier choice via
            # the V8_TO_V9_MAP so an unlimited-tier user doesn't get silently
            # downgraded to a daily-quota model. Custom values (anything not
            # in the map) are left alone.
            elif needs_v9:
                new_model = V8_TO_V9_MAP.get(old_model)
                if new_model is not None and new_model != old_model:
                    gemini_config['model_name'] = new_model
                    logging.info(f'[v9] Bumped Gemini model "{old_model}" -> "{new_model}"')
                    config_changed = True

        # Stamp every version flag up to current so we never re-run on
        # subsequent startups, even if no fields actually needed changing
        # (e.g., a v8 user who'd already picked a custom non-deprecated model).
        for n in range(8, CURRENT_CONFIG_VERSION + 1):
            self.config[f'is_config_file_updated_for_v{n}'] = True

        self.save_config(self.config)
        logging.info('Config migration complete')

        # Single restart popup, regardless of how many versions we jumped.
        if config_changed:
            # Use QMessageBox directly — signals aren't connected yet at this
            # point in __init__.
            QMessageBox.information(
                None,
                'Writing Tools Updated',
                'Writing Tools has just completed an internal update '
                '(your config.json was migrated to the current format).\n\n'
                'Please restart Writing Tools.'
            )
            sys.exit(0)

    def load_options(self):
        """
        Load the options file.
        """
        self.options_path = os.path.join(app_root(), 'options.json')
        logging.debug(f'Loading options from {self.options_path}')
        if os.path.exists(self.options_path):
            with open(self.options_path, 'r') as f:
                self.options = json.load(f)
                logging.debug('Options loaded successfully')
        else:
            logging.debug('Options file not found')
            self.options = None

    def save_config(self, config):
        """
        Save the configuration file.
        """
        with open(self.config_path, 'w') as f:
            json.dump(config, f, indent=4)
            logging.debug('Config saved successfully')
        self.config = config

    def show_onboarding(self):
        """
        Show the onboarding window for first-time users.
        """
        logging.debug('Showing onboarding window')
        self.onboarding_window = ui.OnboardingWindow.OnboardingWindow(self)
        self.onboarding_window.close_signal.connect(self.exit_app)
        self.onboarding_window.show()

    def start_hotkey_listener(self):
        """
        Register the global and direct button shortcuts with the portal.

        Invalid or duplicate triggers are skipped. The first matching trigger
        wins because the portal cannot dispatch it to two shortcut IDs.
        """
        try:
            global_shortcut = self.config.get('shortcut', 'ctrl+space')
            shortcut_map = {'global': global_shortcut}
            taken = {global_shortcut.strip().lower()}

            # Custom actions need typed input and cannot run directly.
            if self.options:
                for button_name, button_cfg in self.options.items():
                    if button_name == 'Custom':
                        continue
                    trigger = (button_cfg.get('hotkey') or '').strip()
                    if not trigger:
                        continue
                    ok, problem = validate_trigger(trigger)
                    if not ok:
                        logging.error(
                            f'Invalid hotkey "{trigger}" for button "{button_name}": {problem}'
                        )
                        continue
                    if trigger.lower() in taken:
                        logging.warning(
                            f'Hotkey "{trigger}" for button "{button_name}" '
                            f'conflicts with an already-registered binding; skipping'
                        )
                        continue
                    taken.add(trigger.lower())
                    shortcut_map['button:' + button_name] = trigger
                    logging.debug(f'Registering button hotkey: {trigger} -> {button_name}')

            self.input_backend.set_callbacks({k: k for k in shortcut_map})
            self.registered_hotkey = None
            self.input_backend.register(shortcut_map)
        except Exception as e:
            logging.error(f'Failed to register global shortcuts: {e}')

    @Slot(str)
    def _fire_button_directly(self, button_name):
        """
        Run a button's option without showing the popup — invoked by a
        per-button direct hotkey. Mirrors the relevant parts of
        `_show_popup`: set up the async clipboard capture, then hand off
        to the worker thread (`process_option`) which waits on the holder
        and dispatches the AI call.
        """
        logging.debug(f'Firing button "{button_name}" directly')

        # If the popup is currently visible (e.g., user opened it then
        # pressed a button hotkey), close it so it doesn't compete.
        if self.popup_window is not None and self.popup_window.isVisible():
            self.popup_window.close()

        # Sanity check: button must still exist in options. Could be stale
        # if options.json was edited externally between registration and
        # fire — skip gracefully rather than crash.
        if not self.options or button_name not in self.options:
            logging.warning(f'Button "{button_name}" no longer exists; ignoring hotkey')
            return

        self.current_text_holder = _SelectedTextHolder()
        self._capture_clipboard_async(self.current_text_holder)

        # Same worker path as a popup-button click. process_option_thread
        # waits on the holder, surfaces "Please select text…" if empty,
        # and routes window-mode options through the response window.
        self.process_option(button_name)

    @Slot(bool)
    def handle_backend_registration(self, registered):
        """Set `registered_hotkey` after portal registration finishes."""
        self.registered_hotkey = self.config.get('shortcut', 'ctrl+space') if registered else None
        if not registered:
            logging.warning('Global shortcut not registered. %s', self.input_backend.diagnostics())

    @Slot(str)
    def handle_backend_shortcut(self, shortcut_id):
        """Handle a portal shortcut on the Qt thread."""
        if self.paused:
            logging.debug(f'Paused; ignoring shortcut "{shortcut_id}"')
            return
        if shortcut_id == 'global':
            self.on_hotkey_pressed()
        elif shortcut_id.startswith('button:'):
            if self.current_provider:
                self.current_provider.cancel()
            self._fire_button_directly(shortcut_id[7:])

    def register_hotkey(self):
        """
        Register the global hotkey for activating Writing Tools.
        """
        logging.debug('Registering hotkey')
        self.start_hotkey_listener()
        logging.debug('Hotkey registered')

    def on_hotkey_pressed(self):
        """
        Handle the hotkey press event.
        """
        logging.debug('Hotkey pressed')
        
        # Check for spam triggers
        if self.check_trigger_spam():
            logging.warning('Hotkey spam detected - quitting application')
            self.exit_app()
            return
            
        # Original hotkey handling continues...
        if self.current_provider:
            logging.debug("Cancelling current provider's request")
            self.current_provider.cancel()

        # noinspection PyTypeChecker
        QtCore.QMetaObject.invokeMethod(self, "_show_popup", QtCore.Qt.ConnectionType.QueuedConnection)

    @Slot()
    def _show_popup(self):
        """
        Show the popup window the moment the hotkey fires, and capture the
        user's selected text in parallel — popup display no longer waits on
        the clipboard. The old behaviour gated popup show on a 0.2-0.5s
        clipboard read, which on slower systems would time out and
        incorrectly fall back to the chat-only "Ask your AI" UI even when
        text *was* selected. We now assume text is always selected;
        `process_option_thread` waits on the holder before kicking off the
        AI request.
        """
        logging.debug('Showing popup window')

        self.current_text_holder = _SelectedTextHolder()
        self._capture_clipboard_async(self.current_text_holder)

        try:
            if self.popup_window is not None:
                logging.debug('Existing popup window found')
                # Clear the attribute first: close() can re-enter through Qt's
                # event loop, and the old window must already be unreachable by
                # then. Always close *and* delete -- a merely hidden popup keeps
                # its pending focus timer alive and leaks.
                stale_popup, self.popup_window = self.popup_window, None
                stale_popup.close()
                stale_popup.deleteLater()
            logging.debug('Creating new popup window')
            self.popup_window = ui.CustomPopupWindow.CustomPopupWindow(self)

            # Set the window icon
            icon_path = os.path.join(asset_root(), 'icons', 'app_icon.png')
            if os.path.exists(icon_path): self.setWindowIcon(QtGui.QIcon(icon_path))
            # Show the popup
            self.popup_window.show()
            self.popup_window.adjustSize()
            # Ensure the popup it's focused, even on lower-end machines
            self.popup_window.activateWindow()
            QtCore.QTimer.singleShot(100, self.popup_window.custom_input.setFocus)

            # Wayland clients cannot position top-level windows.
            logging.debug('Leaving popup placement to the compositor')
        except Exception as e:
            logging.error(f'Error showing popup window: {e}', exc_info=True)

    def _capture_clipboard_async(self, holder):
        """
        Read the clipboard without delaying the popup.

        Wayland cannot read another application's selection, so users must
        copy text before invoking Writing Tools.
        """
        def read():
            with self._capture_lock:
                holder.text = self.input_backend.read_clipboard()
                logging.debug(f'Captured clipboard text (len={len(holder.text)})')
                holder.ready.set()

        threading.Thread(target=read, daemon=True).start()

    def process_option(self, option, custom_change=None):
        """
        Spawn a worker thread that waits for the asynchronous clipboard
        capture and then runs the chosen option. Kept as a thin wrapper so
        the popup's click handler returns immediately and the GUI thread
        is never blocked on the clipboard read.
        """
        logging.debug(f'Processing option: {option}')

        # Drop any stale ref so a previous run's late-arriving response can't
        # land in a now-irrelevant window. The new window (if any) is created
        # by the worker via `_setup_response_window` once the text is in.
        if hasattr(self, 'current_response_window'):
            delattr(self, 'current_response_window')

        threading.Thread(
            target=self.process_option_thread,
            args=(option, custom_change),
            daemon=True
        ).start()

    @Slot(str, str)
    def _setup_response_window(self, option, selected_text):
        """
        Open the response window and seed its chat history. Called from
        `process_option_thread` via `BlockingQueuedConnection` so the
        worker can rely on the window existing before it dispatches the
        AI request.
        """
        self.current_response_window = self.show_response_window(option, selected_text)
        self.current_response_window.chat_history = [
            {
                "role": "user",
                "content": f"Original text to {option.lower()}:\n\n{selected_text}"
            }
        ]

    def process_option_thread(self, option, custom_change=None):
        """
        Worker: wait for the background clipboard capture to land, then
        either open a response window (for window-mode options) or set up
        for clipboard output, and finally run the AI request.
        """
        logging.debug(f'Starting processing thread for option: {option}')

        # Typically near-instant since the user took time to read the popup
        # and click. The 3s ceiling is a safety net for genuinely sluggish
        # systems; if the 2s polling deadline in the capture thread tripped
        # first, the event is already set and this returns immediately.
        holder = self.current_text_holder
        if holder is None or not holder.ready.wait(timeout=3.0):
            logging.warning('Timed out waiting for selected text capture')
        selected_text = (holder.text if holder else '') or ''

        if not selected_text.strip():
            # The chat-mode fallback that used to fire here was removed when
            # popup show became instant — we no longer have a way to detect
            # "user wants to chat" vs "capture failed", so we pick the safer
            # interpretation and surface the error.
            detail = self.input_backend.diagnostics()
            message = 'Copy the text you want to use before invoking Writing Tools.'
            if detail:
                message += '\n\n' + detail
            self.show_message_signal.emit('Error', message)
            return

        if self.options[option]['open_in_window']:
            QtCore.QMetaObject.invokeMethod(
                self,
                '_setup_response_window',
                QtCore.Qt.ConnectionType.BlockingQueuedConnection,
                QtCore.Q_ARG(str, option),
                QtCore.Q_ARG(str, selected_text)
            )

        try:
            selected_prompt = self.options.get(option, ('', ''))
            prompt_prefix = selected_prompt['prefix']
            system_instruction = selected_prompt['instruction']
            if option == 'Custom':
                prompt = f"{prompt_prefix}Described change: {custom_change}\n\nText: {selected_text}"
            else:
                prompt = f"{prompt_prefix}{selected_text}"

            logging.debug(f'Getting response from provider for option: {option}')

            if self.options[option]['open_in_window']:
                logging.debug('Getting response for window display')
                response = self.current_provider.get_response(system_instruction, prompt, return_response=True)
                logging.debug(f'Got response of length: {len(response) if response else 0}')

                if hasattr(self, 'current_response_window'):
                    # noinspection PyTypeChecker
                    QtCore.QMetaObject.invokeMethod(
                        self.current_response_window,
                        'set_text',
                        QtCore.Qt.ConnectionType.QueuedConnection,
                        QtCore.Q_ARG(str, response)
                    )
                    logging.debug('Invoked set_text on response window')
            else:
                logging.debug('Getting response for clipboard output')
                self.current_provider.get_response(system_instruction, prompt)
                logging.debug('Response processed')

        except Exception as e:
            logging.error(f'An error occurred: {e}', exc_info=True)

            if "Resource has been exhausted" in str(e):
                self.show_message_signal.emit('Error - Rate Limit Hit', 'Whoops! You\'ve hit the per-minute rate limit of the Gemini API. Please try again in a few moments.\n\nIf this happens often, simply switch to a Gemini model with a higher usage limit in Settings.')
            else:
                self.show_message_signal.emit('Error', f'An error occurred: {e}')

    @Slot(str, str)
    def show_message_box(self, title, message):
        """
        Show a message box with the given title and message.
        """
        QMessageBox.warning(None, title, message)

    def show_response_window(self, option, text):
        """
        Show the response in a new window.
        """
        response_window = ui.ResponseWindow.ResponseWindow(self, f"{option} Result")
        response_window.selected_text = text  # Store the text for regeneration
        response_window.show()
        return response_window

    @Slot(str)
    def handle_output_ready(self, new_text):
        """Copy a complete response and signal when it is ready to paste."""
        if not isinstance(new_text, str) or not new_text.strip():
            logging.warning(f'Discarding empty response from provider (type={type(new_text).__name__})')
            self.show_message_signal.emit(
                'Empty Response',
                'The AI returned an empty response, so nothing was copied. Please try again.'
            )
            return
        if new_text.strip() == 'ERROR_TEXT_INCOMPATIBLE_WITH_REQUEST':
            self.show_message_signal.emit('Error', 'The text is incompatible with the requested change.')
            return

        # Mirrors the providers' own `hasattr` guard. They decide whether to
        # emit on a worker thread; by the time this queued slot runs on the GUI
        # thread a window-mode run may have opened a window, and that window
        # owns its own display path (`set_text`). Drop the stale result rather
        # than clobber the clipboard behind the user's back.
        if hasattr(self, 'current_response_window'):
            logging.debug('Dropping clipboard response: a response window is active')
            return

        if not self.input_backend.write_clipboard(new_text.rstrip('\n')):
            self.show_message_signal.emit(
                'Clipboard Error',
                'Could not copy the response. Check that wl-clipboard is installed and the Wayland clipboard is available.'
            )
            return
        self.show_clipboard_ready()

    def show_clipboard_ready(self):
        """Restart the brief ready indicator after each successful copy."""
        if self.tray_icon is None:
            return
        self.tray_icon.setIcon(self.ready_tray_icon)
        self.tray_icon.setToolTip(self._('WritingTools — Ready to paste'))
        self.tray_ready_timer.start(5000)

    def restore_tray_icon(self):
        if self.tray_icon is None:
            return
        # Restores whatever the tray started with. `create_tray_icon` has
        # already substituted a themed icon if the bundled PNG was missing, so
        # a still-null icon here just means "back to the startup appearance".
        self.tray_icon.setIcon(self.normal_tray_icon)
        self.tray_icon.setToolTip("WritingTools")

    def create_tray_icon(self):
        """
        Create the system tray icon for the application.
        """
        if self.tray_icon:
            logging.debug('Tray icon already exists')
            return

        logging.debug('Creating system tray icon')
        icon_path = os.path.join(asset_root(), 'icons', 'app_icon.png')
        if not os.path.exists(icon_path):
            logging.warning(f'Tray icon not found at {icon_path}')
            # Use a default icon if not found
            self.tray_icon = QtWidgets.QSystemTrayIcon(self)
        else:
            self.tray_icon = QtWidgets.QSystemTrayIcon(QtGui.QIcon(icon_path), self)
        self.normal_tray_icon = self.tray_icon.icon()
        if self.normal_tray_icon.isNull():
            self.normal_tray_icon = QtGui.QIcon.fromTheme('accessories-text-editor')
        self.ready_tray_icon = QtGui.QIcon(os.path.join(asset_root(), 'icons', 'clipboard_ready.svg'))
        self.tray_ready_timer = QtCore.QTimer(self)
        self.tray_ready_timer.setSingleShot(True)
        self.tray_ready_timer.timeout.connect(self.restore_tray_icon)
        # Set the tooltip (hover name) for the tray icon
        self.tray_icon.setToolTip("WritingTools")
        self.tray_menu = QtWidgets.QMenu()
        self.tray_icon.setContextMenu(self.tray_menu)

        self.update_tray_menu()
        self.tray_icon.show()
        logging.debug('Tray icon displayed')

    def update_tray_menu(self):
        """
        Update the tray menu with all menu items, including pause functionality
        and proper translations.
        """
        self.tray_menu.clear()

        # Apply dark mode styles using darkdetect
        self.apply_dark_mode_styles(self.tray_menu)

        # Settings menu item
        settings_action = self.tray_menu.addAction(self._('Settings'))
        settings_action.triggered.connect(self.show_settings)

        # Pause/Resume toggle action 
        self.toggle_action = self.tray_menu.addAction(self._('Resume') if self.paused else self._('Pause'))
        self.toggle_action.triggered.connect(self.toggle_paused)

        # About menu item
        about_action = self.tray_menu.addAction(self._('About'))
        about_action.triggered.connect(self.show_about)

        # Exit menu item
        exit_action = self.tray_menu.addAction(self._('Exit'))
        exit_action.triggered.connect(self.exit_app)
        
    def toggle_paused(self):
        """Toggle the paused state of the application."""
        logging.debug('Toggle paused state')
        self.paused = not self.paused
        self.toggle_action.setText(self._('Resume') if self.paused else self._('Pause'))
        logging.debug('App is paused' if self.paused else 'App is resumed')

    @staticmethod
    def apply_dark_mode_styles(menu):
        """
        Apply styles to the tray menu based on system theme using darkdetect.
        """
        is_dark_mode = darkdetect.isDark()
        palette = menu.palette()

        if is_dark_mode:
            logging.debug('Tray icon dark')
            # Dark mode colors
            palette.setColor(QtGui.QPalette.Window, QtGui.QColor("#2d2d2d"))  # Dark background
            palette.setColor(QtGui.QPalette.WindowText, QtGui.QColor("#ffffff"))  # White text
        else:
            logging.debug('Tray icon light')
            # Light mode colors
            palette.setColor(QtGui.QPalette.Window, QtGui.QColor("#ffffff"))  # Light background
            palette.setColor(QtGui.QPalette.WindowText, QtGui.QColor("#000000"))  # Black text

        menu.setPalette(palette)


    """
    The function below (process_followup_question) processes follow-up questions in the chat interface for Summary, Key Points, and Table operations.

    This method handles the complex interaction between the UI, chat history, and AI providers:

    1. Chat History Management:
    - Maintains a list of all messages (original text, summary, follow-ups)
    - Properly formats roles (user/assistant) for each message
    - Preserves conversation context across multiple questions (until the Window is closed)

    2. Provider-Specific Handling:
    a) Gemini:
        - Converts internal roles to Gemini's user/model format
        - Uses chat sessions with proper history formatting
        - Maintains context through chat.send_message()
    
    b) OpenAI-compatible:
        - Uses standard OpenAI message array format
        - Includes system instruction and full conversation history
        - Properly maps internal roles to OpenAI roles

    3. Flow:
    a) User asks follow-up question
    b) Question is added to chat history
    c) Full history is formatted for the current provider
    d) Response is generated while maintaining context
    e) Response is displayed in chat UI
    f) New response is added to history for future context

    4. Threading:
    - Runs in a separate thread to prevent UI freezing
    - Uses signals to safely update UI from background thread
    - Handles errors too

    Args:
        response_window: The ResponseWindow instance managing the chat UI
        question: The follow-up question from the user

    This implementation is a bit convoluted, but it allows us to manage chat history & model roles across both providers! :3
    """

    def process_followup_question(self, response_window, question):
        """
        Process a follow-up question in the chat window.
        """
        logging.debug(f'Processing follow-up question: {question}')
        
        def process_thread():
            logging.debug('Starting follow-up processing thread')
            try:
                if not response_window.chat_history:
                    logging.error("No chat history found")
                    self.show_message_signal.emit('Error', 'Chat history not found')
                    return

                # Add current question to chat history
                response_window.chat_history.append({
                    "role": "user",
                    "content": question
                })
                
                # Get chat history
                history = response_window.chat_history.copy()
                
                # System instruction based on original option
                system_instruction = "You are a helpful AI assistant. Provide clear and direct responses, maintaining the same format and style as your previous responses. If appropriate, use Markdown formatting to make your response more readable."
                
                logging.debug('Sending request to AI provider')
                
                # Format conversation differently based on provider
                if isinstance(self.current_provider, GeminiProvider):
                    # Gemini takes the system instruction via its config object,
                    # not as an in-history message. We pass the raw chat history
                    # (user/assistant turns); GeminiProvider handles role mapping
                    # and drops any "system" entries internally.
                    response_text = self.current_provider.get_response(
                        system_instruction,
                        history,
                        return_response=True
                    )

                elif isinstance(self.current_provider, OllamaProvider):  #
                    # For Ollama, prepare messages with system instruction and history
                    messages = [{"role": "system", "content": system_instruction}]

                    for msg in history:
                        messages.append({
                            "role": msg["role"],
                            "content": msg["content"]
                        })

                    # Get response from Ollama
                    response_text = self.current_provider.get_response(
                        system_instruction,
                        messages,
                        return_response=True
                    )

                else:
                    # For OpenAI/compatible providers, prepare messages array, add system message
                    messages = [{"role": "system", "content": system_instruction}]

                    # Add history messages (including latest question)
                    for msg in history:
                        # Convert 'assistant' role to 'assistant' for OpenAI
                        role = "assistant" if msg["role"] == "assistant" else "user"
                        messages.append({"role": role, "content": msg["content"]})
                    
                    # Get response by passing the full messages array
                    response_text = self.current_provider.get_response(
                        system_instruction,
                        messages,  # Pass messages array directly
                        return_response=True
                    )

                logging.debug(f'Got response of length: {len(response_text)}')
                
                # Add response to chat history
                response_window.chat_history.append({
                    "role": "assistant",
                    "content": response_text
                })
                
                # Emit response via signal
                self.followup_response_signal.emit(response_text)

            except Exception as e:
                logging.error(f'Error processing follow-up question: {e}', exc_info=True)

                if "Resource has been exhausted" in str(e):
                    self.show_message_signal.emit('Error - Rate Limit Hit', 'Whoops! You\'ve hit the per-minute rate limit of the Gemini API. Please try again in a few moments.\n\nIf this happens often, simply switch to a Gemini model with a higher usage limit in Settings.')
                    self.followup_response_signal.emit("Sorry, an error occurred while processing your question.")
                else:
                    self.show_message_signal.emit('Error', f'An error occurred: {e}')
                    self.followup_response_signal.emit("Sorry, an error occurred while processing your question.")

        # Start the thread
        threading.Thread(target=process_thread, daemon=True).start()

    def show_settings(self, providers_only=False):

        """
        Show the settings window.
        """
        logging.debug('Showing settings window')
        # Always create a new settings window to handle providers_only correctly
        self.settings_window = ui.SettingsWindow.SettingsWindow(self, providers_only=providers_only)
        self.settings_window.close_signal.connect(self.exit_app)
        self.settings_window.retranslate_ui()
        self.settings_window.show()


    def show_about(self):
        """
        Show the about window.
        """
        logging.debug('Showing about window')
        if not self.about_window:
            self.about_window = ui.AboutWindow.AboutWindow()
        self.about_window.show()

    def setup_ctrl_c_listener(self):
        """
        Listener for Ctrl+C to exit the app.
        """
        signal.signal(signal.SIGINT, lambda signum, frame: self.handle_sigint(signum, frame))
        # This empty timer is needed to make sure that the sigint handler gets checked inside the main loop:
        # without it, the sigint handle would trigger only when an event is triggered, either by a hotkey combination
        # or by another GUI event like spawning a new window. With this we trigger it every 100ms with an empy lambda
        # so that the signal handler gets checked regularly.
        self.ctrl_c_timer = QtCore.QTimer()
        self.ctrl_c_timer.start(100)
        self.ctrl_c_timer.timeout.connect(lambda: None)
    def handle_sigint(self, signum, frame):
        """
        Handle the SIGINT signal (Ctrl+C) to exit the app gracefully.
        """
        logging.info("Received SIGINT. Exiting...")
        self.exit_app()

    def exit_app(self):
        """
        Exit the application.
        """
        logging.debug('Stopping the listener')
        self.input_backend.stop()
        for provider in self.providers:
            try:
                provider.shutdown()
            except Exception:
                logging.exception("Failed to shut down provider %s", provider.provider_name)
        logging.debug('Exiting application')
        self.quit()
