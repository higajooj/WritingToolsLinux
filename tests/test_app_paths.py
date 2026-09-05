import os
from pathlib import Path
import shlex
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from app_paths import app_root, asset_root, codex_data_root
from platform_input import desktop_entry_contents


class AppPathTests(unittest.TestCase):
    def test_codex_data_uses_xdg_data_home(self):
        with tempfile.TemporaryDirectory() as directory:
            old_value = os.environ.get("XDG_DATA_HOME")
            try:
                os.environ["XDG_DATA_HOME"] = directory
                self.assertEqual(
                    codex_data_root(),
                    Path(directory) / "com.writingtools.WritingTools" / "codex",
                )
            finally:
                if old_value is None:
                    os.environ.pop("XDG_DATA_HOME", None)
                else:
                    os.environ["XDG_DATA_HOME"] = old_value

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
