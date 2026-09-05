"""Source and packaged launch paths must not depend on the working directory."""

import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from app_paths import app_root, asset_root
from platform_input import desktop_entry_contents


class AppPathTests(unittest.TestCase):
    def test_source_paths_from_another_directory(self):
        root = Path(__file__).resolve().parents[1]
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as directory:
            try:
                os.chdir(directory)
                with patch.object(sys, "frozen", False, create=True):
                    self.assertEqual(app_root(), root)
                    self.assertEqual(asset_root(), root / "assets")
                    self.assertTrue((app_root() / "options.json").is_file())
                    self.assertTrue((asset_root() / "icons/app_icon.png").is_file())
                    entry = desktop_entry_contents()
                    self.assertIn(shlex.quote(str(root / "main.py")), entry)
                    self.assertIn(f"Icon={root}/assets/icons/app_icon.png", entry)
            finally:
                os.chdir(previous)

    def test_frozen_paths_and_desktop_entry(self):
        with tempfile.TemporaryDirectory(prefix="writing tools ") as directory:
            root = Path(directory).resolve()
            executable = root / "Writing Tools"
            (root / "icons").mkdir()
            (root / "icons/app_icon.png").touch()
            with patch.object(sys, "frozen", True, create=True), patch.object(
                sys, "executable", str(executable)
            ):
                self.assertEqual(app_root(), root)
                self.assertEqual(asset_root(), root)
                entry = desktop_entry_contents()
                self.assertIn(f"Exec={shlex.quote(str(executable))}\n", entry)
                self.assertIn(f"Icon={root}/icons/app_icon.png", entry)


if __name__ == "__main__":
    unittest.main()
