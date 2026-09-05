"""Platform input and clipboard adapters.

Wayland intentionally does not expose the X11 global input APIs.  This module
keeps those details out of the application workflow and provides a graceful
clipboard-first path when a compositor cannot inject input.
"""

import logging
import os
import shutil
import subprocess
import threading
import time

from PySide6 import QtCore

APP_ID = "com.writingtools.WritingTools"


class InputBackend:
    capabilities = {
        "global_shortcuts": False,
        "automatic_selection_capture": False,
        "automatic_paste": False,
    }

    def __init__(self, app):
        self.app = app

    def register(self, shortcut_map):
        raise NotImplementedError

    def stop(self):
        pass

    def capture_selected_text(self, holder):
        """Capture selected text, setting holder.text and holder.ready."""
        holder.ready.set()

    def read_clipboard(self):
        raise NotImplementedError

    def write_clipboard(self, text):
        raise NotImplementedError

    def paste(self):
        return False

    def diagnostics(self):
        return ""


class WaylandInputBackend(InputBackend):
    """Wayland clipboard backend with optional Hyprland paste injection."""

    def __init__(self, app):
        super().__init__(app)
        self._portal = None
        self._portal_session = None
        self._portal_thread = None
        self._portal_error = None
        self._has_wl_clipboard = bool(shutil.which("wl-paste") and shutil.which("wl-copy"))
        self._has_hyprctl = bool(shutil.which("hyprctl")) and os.environ.get("XDG_CURRENT_DESKTOP", "").lower().find("hyprland") >= 0
        self.capabilities = {
            "global_shortcuts": False,
            "automatic_selection_capture": False,
            "automatic_paste": self._has_hyprctl,
            "clipboard": self._has_wl_clipboard,
        }

    def register(self, shortcut_map):
        """Register shortcuts through the portal when dbus-next is available.

        Portal APIs are asynchronous and intentionally optional: systems with
        no GlobalShortcuts implementation still get clipboard-first operation.
        """
        try:
            from dbus_next import BusType, Message, MessageType
            from dbus_next.aio import MessageBus
        except ImportError:
            self._portal_error = "Install dbus-next to enable Wayland global shortcuts."
            logging.warning(self._portal_error)
            return

        self._portal_thread = threading.Thread(
            target=self._run_portal, args=(shortcut_map, BusType, Message, MessageType, MessageBus), daemon=True
        )
        self._portal_thread.start()

    def _run_portal(self, shortcut_map, BusType, Message, MessageType, MessageBus):
        import asyncio
        from dbus_next import Variant

        async def run():
            try:
                bus = await MessageBus(bus_type=BusType.SESSION).connect()
                self._portal = bus
                reply = await bus.call(Message(
                    destination="org.freedesktop.portal.Desktop",
                    path="/org/freedesktop/portal/desktop",
                    interface="org.freedesktop.portal.GlobalShortcuts",
                    member="CreateSession",
                    signature="a{sv}",
                    body=[{
                        "handle_token": Variant("s", "writingtools"),
                        "session_handle_token": Variant("s", "writingtools"),
                        "app_id": Variant("s", APP_ID),
                    }],
                ))
                if reply.message_type != MessageType.METHOD_RETURN:
                    raise RuntimeError(str(reply.body))
                request_path = reply.body[0]
                session_path = await self._request_response(bus, request_path)
                self._portal_session = session_path
                shortcuts = []
                for shortcut_id, trigger in shortcut_map.items():
                    shortcuts.append((shortcut_id, {
                        "description": Variant("s", "Writing Tools: " + shortcut_id),
                        "preferred_trigger": Variant("s", self._portal_trigger(trigger)),
                    }))
                reply = await bus.call(Message(
                    destination="org.freedesktop.portal.Desktop",
                    path="/org/freedesktop/portal/desktop",
                    interface="org.freedesktop.portal.GlobalShortcuts",
                    member="BindShortcuts",
                    signature="oa(sa{sv})sa{sv}",
                    body=[session_path, shortcuts, "", {}],
                ))
                request_path = reply.body[0]
                await self._request_response(bus, request_path)
                self.capabilities["global_shortcuts"] = True
                bus.add_message_handler(self._portal_signal)
                await asyncio.Future()
            except Exception as exc:
                self._portal_error = str(exc)
                logging.warning("Wayland GlobalShortcuts unavailable: %s", exc)

        asyncio.run(run())

    async def _request_response(self, bus, request_path):
        import asyncio
        loop = asyncio.get_running_loop()
        future = loop.create_future()

        def handler(message):
            if message.path == request_path and message.member == "Response" and not future.done():
                future.set_result(message.body[1] if message.body and message.body[0] == 0 else None)
                return None
            return None

        bus.add_message_handler(handler)
        result = await future
        if result is None:
            raise RuntimeError("Wayland portal request was cancelled or rejected")
        if isinstance(result, dict):
            result = result.get("session_handle", result)
        return getattr(result, "value", result)

    @staticmethod
    def _portal_trigger(trigger):
        """Convert the settings syntax to the XDG shortcut trigger syntax."""
        names = {"ctrl": "CTRL", "alt": "ALT", "shift": "SHIFT", "super": "SUPER", "meta": "SUPER"}
        parts = [part.strip().lower() for part in trigger.split("+") if part.strip()]
        if not parts:
            return trigger
        converted = [names.get(part, part.upper()) for part in parts]
        return "+".join(converted)

    def _portal_signal(self, message):
        if message.member != "Activated" or len(message.body) < 2:
            return
        shortcut_id = message.body[1]
        if shortcut_id in getattr(self, "_shortcut_callbacks", {}):
            QtCore.QMetaObject.invokeMethod(
                self.app,
                "handle_backend_shortcut",
                QtCore.Qt.ConnectionType.QueuedConnection,
                QtCore.Q_ARG(str, shortcut_id),
            )

    def set_callbacks(self, callbacks):
        self._shortcut_callbacks = callbacks

    def read_clipboard(self):
        if not self._has_wl_clipboard:
            return ""
        try:
            result = subprocess.run(["wl-paste", "--no-newline"], capture_output=True, text=True, timeout=2)
            return result.stdout if result.returncode == 0 else ""
        except (OSError, subprocess.SubprocessError):
            return ""

    def write_clipboard(self, text):
        if not self._has_wl_clipboard:
            return False
        try:
            result = subprocess.run(["wl-copy"], input=text, text=True, timeout=2)
            return result.returncode == 0
        except (OSError, subprocess.SubprocessError):
            return False

    def paste(self):
        if not self._has_hyprctl:
            return False
        try:
            result = subprocess.run(
                ["hyprctl", "dispatch", "sendshortcut", "CTRL,V", "activewindow"],
                capture_output=True, text=True, timeout=2,
            )
            return result.returncode == 0
        except (OSError, subprocess.SubprocessError):
            return False

    def diagnostics(self):
        problems = []
        if not self._has_wl_clipboard:
            problems.append("Install wl-clipboard (wl-copy and wl-paste) for clipboard workflows.")
        if self._portal_error:
            problems.append(self._portal_error)
        if not self._has_hyprctl:
            problems.append("Automatic paste is unavailable outside Hyprland; paste the result manually.")
        return " ".join(problems)


class X11InputBackend(InputBackend):
    """Adapter for the existing pynput/pyperclip implementation."""

    def __init__(self, app, keyboard, clipboard):
        super().__init__(app)
        self.keyboard = keyboard
        self.clipboard = clipboard
        self.capabilities = {
            "global_shortcuts": True,
            "automatic_selection_capture": True,
            "automatic_paste": True,
            "clipboard": True,
        }

    def read_clipboard(self):
        return self.clipboard.paste()

    def write_clipboard(self, text):
        self.clipboard.copy(text)
        return True

    def paste(self):
        keyboard = self.keyboard.Controller()
        keyboard.press(self.keyboard.Key.ctrl.value)
        keyboard.press("v")
        keyboard.release("v")
        keyboard.release(self.keyboard.Key.ctrl.value)
        return True
