"""Wayland input and clipboard support."""

import logging
import os
import shlex
import shutil
import subprocess
import sys
import threading

from app_paths import app_root, asset_root
from PySide6 import QtCore

APP_ID = "com.writingtools.WritingTools"
PORTAL_BUS = "org.freedesktop.portal.Desktop"
PORTAL_PATH = "/org/freedesktop/portal/desktop"
SHORTCUTS_IFACE = "org.freedesktop.portal.GlobalShortcuts"
MODIFIER_NAMES = {
    "ctrl": "CTRL", "control": "CTRL", "alt": "ALT", "shift": "SHIFT",
    "super": "SUPER", "meta": "SUPER", "win": "SUPER", "cmd": "SUPER",
}


def validate_trigger(trigger):
    """Validate a shortcut before passing it to the portal."""
    parts = [part.strip() for part in (trigger or "").split("+")]
    if not parts or any(not part for part in parts):
        return False, "Separate keys with '+', for example ctrl+space."
    modifiers = [part for part in parts if part.lower() in MODIFIER_NAMES]
    keys = [part for part in parts if part.lower() not in MODIFIER_NAMES]
    if not modifiers:
        return False, "Add a modifier, for example ctrl+space."
    if len(keys) != 1:
        return False, "Use one key with the modifiers, for example super+p."
    return True, ""


def desktop_entry_path():
    data_home = os.environ.get("XDG_DATA_HOME") or os.path.join(os.path.expanduser("~"), ".local", "share")
    return os.path.join(data_home, "applications", APP_ID + ".desktop")


def desktop_entry_contents():
    # Resolving the interpreter path would bypass an active virtual environment.
    exec_line = "{} {}".format(
        shlex.quote(os.path.abspath(sys.executable)),
        shlex.quote(str(app_root() / "main.py")),
    )
    icon = str(asset_root() / "icons" / "app_icon.png")
    lines = [
        "[Desktop Entry]",
        "Type=Application",
        "Name=Writing Tools",
        "Comment=AI writing assistant",
        "Exec=" + exec_line,
    ]
    if os.path.exists(icon):
        lines.append("Icon=" + icon)
    lines += ["Terminal=false", "Categories=Utility;", "StartupWMClass=" + APP_ID, ""]
    return "\n".join(lines)


def ensure_desktop_entry():
    """Create the desktop entry required by the portal."""
    path = desktop_entry_path()
    if os.path.exists(path):
        return ""
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as entry:
            entry.write(desktop_entry_contents())
    except OSError as exc:
        message = "Could not create desktop entry at {} ({}). Wayland global shortcuts require it.".format(path, exc)
        logging.warning(message)
        return message
    logging.info("Installed desktop entry %s for portal app ID %s", path, APP_ID)
    return ""


class WaylandInputBackend:
    """Wayland clipboard backend with optional Hyprland paste injection."""

    def __init__(self, app):
        self.app = app
        self._portal_bus = None
        self._portal_session = None
        self._portal_thread = None
        self._portal_loop = None
        self._portal_closed = None
        self._portal_error = None
        self._entry_error = ""
        self._portal_token = 0
        self._bound_shortcuts = ()
        self._shortcut_callbacks = {}
        self._has_wl_clipboard = bool(shutil.which("wl-paste") and shutil.which("wl-copy"))
        self._is_hyprland = os.environ.get("XDG_CURRENT_DESKTOP", "").lower().find("hyprland") >= 0
        self._has_hyprctl = bool(shutil.which("hyprctl")) and self._is_hyprland
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
        # The portal requires a desktop entry for this app ID. Install it even
        # when dbus-next is missing: Qt's own portal registration needs it too.
        self._entry_error = ensure_desktop_entry()

        try:
            from dbus_next import BusType, Message, MessageType
            from dbus_next.aio import MessageBus
        except ImportError:
            self._portal_error = "Install dbus-next to enable Wayland global shortcuts."
            logging.warning(self._portal_error)
            return

        # Re-registering closes the previous portal session.
        self.stop()
        self._portal_error = None
        self._portal_thread = threading.Thread(
            target=self._run_portal,
            args=(dict(shortcut_map), BusType, Message, MessageType, MessageBus),
            daemon=True,
        )
        self._portal_thread.start()

    def stop(self):
        thread = self._portal_thread
        loop = self._portal_loop
        closed = self._portal_closed
        if loop is not None and closed is not None:
            loop.call_soon_threadsafe(self._resolve_closed, closed)
        if thread is not None and thread.is_alive() and thread is not threading.current_thread():
            thread.join(timeout=2)
        self._portal_thread = None
        self._portal_loop = None
        self._portal_closed = None
        self._portal_session = None
        self._bound_shortcuts = ()
        self.capabilities["global_shortcuts"] = False

    @staticmethod
    def _resolve_closed(closed):
        if not closed.done():
            closed.set_result(None)

    def _run_portal(self, shortcut_map, BusType, Message, MessageType, MessageBus):
        import asyncio
        from dbus_next import Variant

        async def run():
            bus = None
            try:
                bus = await MessageBus(bus_type=BusType.SESSION).connect()
                self._portal_bus = bus
                self._portal_loop = asyncio.get_running_loop()
                self._portal_closed = self._portal_loop.create_future()

                # Registry.Register must be the first portal call.
                await self._register_app_id(bus, Message, MessageType)

                results = await self._portal_request(
                    bus, Message, MessageType, "CreateSession", "a{sv}",
                    lambda token: [{
                        "handle_token": Variant("s", token),
                        "session_handle_token": Variant("s", token),
                    }],
                )
                session_path = self._unwrap(results.get("session_handle"))
                if not session_path:
                    raise RuntimeError("Portal returned no session handle")
                self._portal_session = session_path

                # dbus-next sends a D-Bus struct from a list, not a tuple.
                shortcuts = [
                    [shortcut_id, {
                        "description": Variant("s", "Writing Tools: " + shortcut_id),
                        "preferred_trigger": Variant("s", self._portal_trigger(trigger)),
                    }]
                    for shortcut_id, trigger in shortcut_map.items()
                ]
                await self._portal_request(
                    bus, Message, MessageType, "BindShortcuts", "oa(sa{sv})sa{sv}",
                    lambda token: [session_path, shortcuts, "", {"handle_token": Variant("s", token)}],
                )

                self._bound_shortcuts = tuple(shortcut_map.items())
                self.capabilities["global_shortcuts"] = True
                bus.add_message_handler(self._portal_signal)
                logging.info(
                    "Wayland GlobalShortcuts bound for %s: %s",
                    APP_ID, ", ".join(shortcut_map),
                )
                hint = self.compositor_bind_hint()
                if hint:
                    logging.info(hint)
                self._notify_registration(True)
                await self._portal_closed
            except Exception as exc:
                self._portal_error = str(exc)
                self.capabilities["global_shortcuts"] = False
                logging.warning("Wayland GlobalShortcuts unavailable: %s", exc)
                self._notify_registration(False)
            finally:
                if bus is not None:
                    if self._portal_session:
                        try:
                            reply = await bus.call(Message(
                                destination=PORTAL_BUS,
                                path=self._portal_session,
                                interface="org.freedesktop.portal.Session",
                                member="Close",
                            ))
                            logging.debug(
                                "Portal session closed: %s",
                                "ok" if reply.message_type == MessageType.METHOD_RETURN else self._error_text(reply),
                            )
                        except Exception:
                            logging.debug("Portal session close failed", exc_info=True)
                    bus.disconnect()
                self._portal_bus = None

        asyncio.run(run())

    async def _register_app_id(self, bus, Message, MessageType):
        """Register this D-Bus connection as Writing Tools.

        Shell-launched processes may have no app ID. Older portals lack the
        Registry interface, so a registration failure does not stop setup.
        """
        reply = await bus.call(Message(
            destination=PORTAL_BUS,
            path=PORTAL_PATH,
            interface="org.freedesktop.host.portal.Registry",
            member="Register",
            signature="sa{sv}",
            body=[APP_ID, {}],
        ))
        if reply.message_type != MessageType.METHOD_RETURN:
            logging.info("Portal app id registration skipped: %s", self._error_text(reply))
        else:
            logging.debug("Registered portal app id %s", APP_ID)

    async def _portal_request(self, bus, Message, MessageType, member, signature, body_factory):
        import asyncio

        loop = asyncio.get_running_loop()
        future = loop.create_future()
        self._portal_token += 1
        token = "writingtools{}".format(self._portal_token)
        paths = {self._request_path(bus.unique_name, token)}

        def handler(message):
            if message.member == "Response" and message.path in paths and not future.done():
                future.set_result(message.body)
            return None

        # The portal can emit Response before the method reply arrives.
        bus.add_message_handler(handler)
        try:
            reply = await bus.call(Message(
                destination=PORTAL_BUS,
                path=PORTAL_PATH,
                interface=SHORTCUTS_IFACE,
                member=member,
                signature=signature,
                body=body_factory(token),
            ))
            if reply.message_type != MessageType.METHOD_RETURN:
                raise RuntimeError(self._error_text(reply))
            if reply.body:
                paths.add(reply.body[0])
            body = await future
        finally:
            bus.remove_message_handler(handler)

        response = body[0] if body else 1
        if response != 0:
            raise RuntimeError(
                "{} was cancelled or rejected by the portal (response {})".format(member, response)
            )
        results = body[1] if len(body) > 1 else {}
        return results if isinstance(results, dict) else {}

    @staticmethod
    def _request_path(unique_name, token):
        sender = (unique_name or "").lstrip(":").replace(".", "_")
        return "/org/freedesktop/portal/desktop/request/{}/{}".format(sender, token)

    @staticmethod
    def _unwrap(value):
        return getattr(value, "value", value)

    @staticmethod
    def _error_text(reply):
        detail = reply.body[0] if reply.body else ""
        name = reply.error_name or "portal error"
        return "{}: {}".format(name, detail) if detail else name

    @staticmethod
    def _portal_trigger(trigger):
        """Keep key case because the portal treats `SUPER+p` and `SUPER+P` differently."""
        parts = [part.strip() for part in trigger.split("+") if part.strip()]
        if not parts:
            return trigger
        return "+".join(MODIFIER_NAMES.get(part.lower(), part) for part in parts)

    def compositor_bind_hint(self):
        """Return Hyprland bindings for shortcuts ignored by preferred_trigger."""
        if not self._is_hyprland or not self._bound_shortcuts:
            return ""
        binds = "; ".join(
            self._hyprland_bind(trigger, shortcut_id)
            for shortcut_id, trigger in self._bound_shortcuts
        )
        return "Hyprland binds portal shortcuts in its own config: " + binds

    @staticmethod
    def _hyprland_bind(trigger, shortcut_id):
        parts = [part.strip() for part in trigger.split("+") if part.strip()]
        modifiers = [MODIFIER_NAMES[part.lower()] for part in parts if part.lower() in MODIFIER_NAMES]
        keys = [part for part in parts if part.lower() not in MODIFIER_NAMES]
        return "bind = {}, {}, global, {}:{}".format(
            " ".join(modifiers), keys[0].upper() if keys else "", APP_ID, shortcut_id
        )

    def _portal_signal(self, message):
        if message.member != "Activated" or message.interface != SHORTCUTS_IFACE:
            return None
        if len(message.body) < 2:
            return None
        shortcut_id = message.body[1]
        if shortcut_id in self._shortcut_callbacks:
            QtCore.QMetaObject.invokeMethod(
                self.app,
                "handle_backend_shortcut",
                QtCore.Qt.ConnectionType.QueuedConnection,
                QtCore.Q_ARG(str, shortcut_id),
            )
        return None

    def _notify_registration(self, registered):
        QtCore.QMetaObject.invokeMethod(
            self.app,
            "handle_backend_registration",
            QtCore.Qt.ConnectionType.QueuedConnection,
            QtCore.Q_ARG(bool, registered),
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
        if self._entry_error:
            problems.append(self._entry_error)
        if self._portal_error:
            problems.append(self._portal_error)
        else:
            hint = self.compositor_bind_hint()
            if hint:
                problems.append(hint)
        if not self._has_hyprctl:
            problems.append("Automatic paste is unavailable outside Hyprland; paste the result manually.")
        return " ".join(problems)
