import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from app_paths import app_root, asset_root
from platform_input import desktop_entry_contents


class AppPathTests(unittest.TestCase):
    def test_paths_from_another_directory(self):
        root = Path(__file__).resolve().parents[1]
        previous = Path.cwd()
        with tempfile.TemporaryDirectory() as directory:
            try:
                os.chdir(directory)
                self.assertEqual(app_root(), root)
                self.assertEqual(asset_root(), root / "assets")
                self.assertTrue((app_root() / "options.json").is_file())
                self.assertTrue((asset_root() / "icons/app_icon.png").is_file())
                entry = desktop_entry_contents()
                self.assertIn(shlex.quote(str(root / "main.py")), entry)
                self.assertIn(f"Icon={root}/assets/icons/app_icon.png", entry)
            finally:
                os.chdir(previous)


if __name__ == "__main__":
    unittest.main()
