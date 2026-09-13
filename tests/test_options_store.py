import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

import options_store


class OptionsStoreTests(unittest.TestCase):
    def setUp(self):
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        root = Path(self.temporary_directory.name)
        self.options_path = root / "options.json"
        self.default_options_path = root / "default_options.json"
        self.defaults = {"Proofread": {"instruction": "default"}}
        self.default_options_path.write_text(
            json.dumps(self.defaults), encoding="utf-8"
        )

        options_patch = patch(
            "options_store.options_path", return_value=self.options_path
        )
        defaults_patch = patch(
            "options_store.default_options_path",
            return_value=self.default_options_path,
        )
        options_patch.start()
        defaults_patch.start()
        self.addCleanup(options_patch.stop)
        self.addCleanup(defaults_patch.stop)

    def test_load_seeds_missing_user_options_from_defaults(self):
        self.assertEqual(options_store.load_options(), self.defaults)
        self.assertEqual(
            json.loads(self.options_path.read_text(encoding="utf-8")),
            self.defaults,
        )

    def test_load_preserves_existing_user_options(self):
        custom = {"Custom button": {"instruction": "personal"}}
        self.options_path.write_text(json.dumps(custom), encoding="utf-8")

        self.assertEqual(options_store.load_options(), custom)
        self.assertEqual(
            json.loads(self.options_path.read_text(encoding="utf-8")), custom
        )

    def test_save_and_reset_target_only_the_user_options_file(self):
        custom = {"Custom button": {"instruction": "personal"}}
        options_store.save_options(custom)
        self.assertEqual(options_store.load_options(), custom)

        self.assertEqual(options_store.reset_options(), self.defaults)
        self.assertEqual(options_store.load_options(), self.defaults)


if __name__ == "__main__":
    unittest.main()
