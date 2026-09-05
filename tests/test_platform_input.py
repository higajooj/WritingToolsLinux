from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from platform_input import validate_trigger


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


if __name__ == "__main__":
    unittest.main()
