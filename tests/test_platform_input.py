from pathlib import Path
import os
import sys
import tempfile
import time
import unittest
from unittest.mock import AsyncMock, Mock

from dbus_next import BusType, Message, MessageType

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from platform_input import WaylandInputBackend, validate_trigger

# Stands in for wl-copy, which forks a daemon to keep owning the selection while
# the process we launched exits straight away. The child inherits our stdio, so a
# caller that pipes stdout/stderr never sees EOF on them.
FORKING_WL_COPY = """#!/usr/bin/env python3
import os, sys, time
sys.stdin.read()
if os.fork():
    sys.exit(0)
time.sleep(3)
"""


class ValidateTriggerTests(unittest.TestCase):
    def test_accepts_portal_triggers(self):
        for trigger in ("ctrl+space", "super+shift+p", "CTRL+J", "alt+F1"):
            with self.subTest(trigger=trigger):
                self.assertEqual(validate_trigger(trigger), (True, ""))

    def test_rejects_malformed_triggers(self):
        for trigger in ("space", "ctrl", "ctrl+", "", "ctrl+a+b", "ctrl+shift"):
            with self.subTest(trigger=trigger):
                ok, message = validate_trigger(trigger)
                self.assertFalse(ok)
                self.assertTrue(message)


class WriteClipboardTests(unittest.TestCase):
    """Guards against piping wl-copy's stdio, which turns a good copy into an error."""

    def setUp(self):
        stubs = tempfile.TemporaryDirectory()
        self.addCleanup(stubs.cleanup)
        bin_dir = Path(stubs.name)
        for name, body in (("wl-copy", FORKING_WL_COPY), ("wl-paste", "#!/bin/sh\nexit 0\n")):
            stub = bin_dir / name
            stub.write_text(body)
            stub.chmod(0o755)

        original_path = os.environ["PATH"]
        self.addCleanup(os.environ.__setitem__, "PATH", original_path)
        os.environ["PATH"] = f"{bin_dir}{os.pathsep}{original_path}"

        self.backend = WaylandInputBackend(app=None)
        self.assertTrue(self.backend.capabilities["clipboard"])

    def test_reports_success_without_waiting_on_the_forked_daemon(self):
        started = time.monotonic()
        copied = self.backend.write_clipboard("Corrected text.")
        elapsed = time.monotonic() - started

        self.assertTrue(copied)
        # Capturing stdout/stderr would block here until the 2s timeout and report
        # a failure for a copy that had in fact succeeded.
        self.assertLess(elapsed, 1.0)


class _FakeBus:
    """Session bus stand-in that records which portal sessions get closed."""

    unique_name = ":1.42"

    def __init__(self, bus_type=None):
        self.closed_sessions = []

    async def connect(self):
        return self

    async def call(self, message):
        if message.member == "Close":
            self.closed_sessions.append(message.path)
        return Mock(message_type=MessageType.METHOD_RETURN, body=[])

    def add_message_handler(self, handler):
        pass

    def disconnect(self):
        pass


class PortalLifecycleTests(unittest.TestCase):
    SHORTCUTS = {"global": "ctrl+space"}

    def setUp(self):
        self.backend = WaylandInputBackend(app=None)
        self.backend._notify_registration = Mock()
        self.backend._register_app_id = AsyncMock()
        self.bus = _FakeBus()

    def run_portal(self):
        self.backend._run_portal(
            self.backend._portal_generation,
            self.SHORTCUTS,
            BusType,
            Message,
            MessageType,
            lambda bus_type: self.bus,
        )

    def test_stop_after_failed_registration_does_not_raise(self):
        self.backend._register_app_id.side_effect = RuntimeError("portal rejected")

        self.run_portal()
        # asyncio.run has closed the run's loop; stop() must not schedule on it.
        self.backend.stop()

        self.assertEqual(self.backend._portal_error, "portal rejected")
        self.backend._notify_registration.assert_called_once_with(False)

    def test_superseded_run_closes_only_its_own_session(self):
        async def portal_request(bus, message_cls, message_type, member, signature, body_factory):
            if member == "CreateSession":
                return {"session_handle": "/org/freedesktop/portal/desktop/session/old"}
            # A newer registration takes over while this bind is pending.
            self.backend.stop()
            self.backend._portal_session = "/org/freedesktop/portal/desktop/session/new"
            return {}

        self.backend._portal_request = portal_request

        self.run_portal()

        self.assertEqual(
            self.bus.closed_sessions,
            ["/org/freedesktop/portal/desktop/session/old"],
        )
        self.assertEqual(
            self.backend._portal_session,
            "/org/freedesktop/portal/desktop/session/new",
        )
        self.assertFalse(self.backend.capabilities["global_shortcuts"])
        self.backend._notify_registration.assert_not_called()


if __name__ == "__main__":
    unittest.main()
