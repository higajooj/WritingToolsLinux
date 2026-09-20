"""SVG icon rendering tests (run Qt offscreen)."""

import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

# Qt must be pinned to the offscreen platform before PySide6 is imported.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

from PySide6 import QtGui, QtWidgets

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from app_paths import asset_root, default_options_path
from ui.UIUtils import UIUtils


def opaque_colors(pixmap):
    image = pixmap.toImage()
    return [
        image.pixelColor(x, y)
        for x in range(image.width())
        for y in range(image.height())
        if image.pixelColor(x, y).alpha() > 0
    ]


class SvgIconTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        # themed_icon memoises on (path, size, color), so a test that swaps the
        # theme must not be served a pixmap another test rendered.
        import ui.UIUtils as ui_utils

        ui_utils._render_svg.cache_clear()
        self.addCleanup(ui_utils._render_svg.cache_clear)

    def test_action_icon_renders_for_both_themes(self):
        for theme in ("light", "dark"):
            with patch("ui.UIUtils.colorMode", theme):
                icon = UIUtils.themed_icon("check")
                pixmap = icon.pixmap(96, 96)
            self.assertFalse(pixmap.isNull())

            colors = opaque_colors(pixmap)
            self.assertTrue(colors)
            expected = QtGui.QColor("white" if theme == "dark" else "black")
            self.assertTrue(
                all(color.rgb() == expected.rgb() for color in colors)
            )

    def test_action_icon_uses_unsuffixed_svg_name(self):
        self.assertFalse(UIUtils.themed_icon("send").isNull())

    def test_stored_icons_prefix_is_accepted(self):
        """Options files store the icon as ``icons/<name>``; so do existing
        user options.json files, so both spellings must resolve."""
        self.assertEqual(
            UIUtils.icon_pixmap("icons/send", 96).toImage(),
            UIUtils.icon_pixmap("send", 96).toImage(),
        )

    def test_missing_asset_yields_a_null_icon(self):
        """Replaces the old ``os.path.exists`` guards at every call site: a
        button with no matching SVG gets no icon rather than raising."""
        self.assertTrue(UIUtils.themed_icon("no-such-icon").isNull())
        self.assertTrue(UIUtils.provider_logo_pixmap("no-such-provider").isNull())

    def test_every_default_option_icon_exists(self):
        options = json.loads(default_options_path().read_text())
        names = {option["icon"] for option in options.values() if option.get("icon")}
        self.assertTrue(names)
        for name in names:
            with self.subTest(icon=name):
                self.assertTrue(
                    (asset_root() / "icons" / f"{name.rsplit('/', 1)[-1]}.svg").is_file()
                )
                self.assertFalse(UIUtils.themed_icon(name).isNull())

    def test_provider_logo_renders_from_svg(self):
        for provider in ("gemini", "ollama", "openai"):
            with self.subTest(provider=provider):
                pixmap = UIUtils.provider_logo_pixmap(provider, 30)
                self.assertFalse(pixmap.isNull())
                self.assertEqual(pixmap.size().toTuple(), (30, 30))

    def test_monochrome_provider_logos_follow_the_theme(self):
        """The Ollama and OpenAI marks are drawn in ``currentColor``; without
        theming they render near-black on the dark settings background."""
        for provider in ("ollama", "openai"):
            for theme in ("light", "dark"):
                with self.subTest(provider=provider, theme=theme):
                    import ui.UIUtils as ui_utils

                    ui_utils._render_svg.cache_clear()
                    with patch("ui.UIUtils.colorMode", theme):
                        pixmap = UIUtils.provider_logo_pixmap(provider, 96)
                    colors = opaque_colors(pixmap)
                    self.assertTrue(colors)
                    expected = QtGui.QColor("white" if theme == "dark" else "black")
                    self.assertTrue(
                        all(color.rgb() == expected.rgb() for color in colors)
                    )

    def test_gemini_logo_keeps_its_own_palette(self):
        """Gemini's gradient carries no ``currentColor``, so theming is a no-op
        for it and the brand colors survive."""
        colors = opaque_colors(UIUtils.provider_logo_pixmap("gemini", 96))
        self.assertTrue(colors)
        self.assertTrue(any(color.red() != color.blue() for color in colors))


if __name__ == "__main__":
    unittest.main()
