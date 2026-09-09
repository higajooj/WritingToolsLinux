from pathlib import Path
import os
import sys
import tempfile
import time
import unittest

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


if __name__ == "__main__":
    unittest.main()
