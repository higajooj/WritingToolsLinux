"""Persistent zoom behavior for response popup windows."""

import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6 import QtCore, QtGui, QtTest, QtWidgets

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from ui.ResponseWindow import (
    DEFAULT_ZOOM_FACTOR,
    MAX_ZOOM_FACTOR,
    MIN_ZOOM_FACTOR,
    ZOOM_STEP,
    MarkdownTextBrowser,
    ResponseWindow,
)


class ResponseWindowZoomTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        self.windows = []

    def tearDown(self):
        for window in self.windows:
            window.thinking_timer.stop()
            window.close()
            window.deleteLater()
        QtWidgets.QApplication.processEvents()

    def make_window(self, config=None):
        app = SimpleNamespace(
            config={} if config is None else config,
            followup_response_signal=Mock(),
            process_followup_question=Mock(),
            save_config=Mock(),
        )
        window = ResponseWindow(app, "Summary Result")
        self.windows.append(window)
        return app, window

    @staticmethod
    def message_displays(window):
        displays = []
        for index in range(window.chat_area.layout.count() - 1):
            container = window.chat_area.layout.itemAt(index).widget()
            display = container.layout().itemAt(0).widget()
            if isinstance(display, MarkdownTextBrowser):
                displays.append(display)
        return displays

    @staticmethod
    def ctrl_wheel_event(angle_delta):
        return QtGui.QWheelEvent(
            QtCore.QPointF(),
            QtCore.QPointF(),
            QtCore.QPoint(),
            angle_delta,
            QtCore.Qt.MouseButton.NoButton,
            QtCore.Qt.KeyboardModifier.ControlModifier,
            QtCore.Qt.ScrollPhase.NoScrollPhase,
            False,
        )

    def test_restores_saved_zoom_for_the_initial_response(self):
        _, window = self.make_window({"response_window_zoom": 1.8})

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Saved zoom response")

        self.assertEqual(window.zoom_factor, 1.8)
        self.assertEqual(self.message_displays(window)[0].zoom_factor, 1.8)

    def test_invalid_saved_zoom_uses_the_default(self):
        for saved_zoom in ("large", None, [], float("nan"), float("inf")):
            with self.subTest(saved_zoom=saved_zoom):
                _, window = self.make_window({"response_window_zoom": saved_zoom})
                self.assertEqual(window.zoom_factor, DEFAULT_ZOOM_FACTOR)

    def test_out_of_range_saved_zoom_is_clamped(self):
        _, large_window = self.make_window({"response_window_zoom": 10})
        _, small_window = self.make_window({"response_window_zoom": 0.1})

        self.assertEqual(large_window.zoom_factor, MAX_ZOOM_FACTOR)
        self.assertEqual(small_window.zoom_factor, MIN_ZOOM_FACTOR)

    def test_zoom_before_first_response_is_saved_and_applied(self):
        app, window = self.make_window()

        window.zoom_all_messages("in")

        expected_zoom = DEFAULT_ZOOM_FACTOR * ZOOM_STEP
        self.assertEqual(window.zoom_factor, expected_zoom)
        self.assertEqual(app.config["response_window_zoom"], expected_zoom)
        self.assertTrue(window.zoom_save_timer.isActive())

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Late response")
        self.assertEqual(self.message_displays(window)[0].zoom_factor, expected_zoom)

    def test_all_conversation_messages_keep_the_active_zoom(self):
        _, window = self.make_window()
        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Initial response")
            window.zoom_all_messages("in")
            window.input_field.setText("Follow-up question")
            window.send_message()
            window.handle_followup_response("Follow-up response")

        self.assertEqual(len(self.message_displays(window)), 3)
        for display in self.message_displays(window):
            self.assertEqual(display.zoom_factor, window.zoom_factor)

    def test_saved_zoom_is_restored_by_the_next_window(self):
        app, first_window = self.make_window()
        first_window.zoom_all_messages("out")

        second_window = ResponseWindow(app, "Rewrite Result")
        self.windows.append(second_window)

        self.assertEqual(second_window.zoom_factor, first_window.zoom_factor)

    def test_reset_saves_the_default_zoom(self):
        app, window = self.make_window({"response_window_zoom": 1.8})

        window.zoom_all_messages("reset")

        self.assertEqual(window.zoom_factor, DEFAULT_ZOOM_FACTOR)
        self.assertEqual(app.config["response_window_zoom"], DEFAULT_ZOOM_FACTOR)
        self.assertTrue(window.zoom_save_timer.isActive())

    def test_rapid_zoom_steps_are_saved_once(self):
        app, window = self.make_window()
        window.zoom_save_timer.setInterval(0)

        for _ in range(3):
            window.zoom_all_messages("in")
        app.save_config.assert_not_called()

        QtTest.QTest.qWait(20)
        app.save_config.assert_called_once_with(app.config)
        self.assertEqual(app.config["response_window_zoom"], window.zoom_factor)

    def test_closing_flushes_a_pending_zoom_save(self):
        app, window = self.make_window()
        window.zoom_all_messages("in")

        window.close()

        app.save_config.assert_called_once_with(app.config)
        self.assertFalse(window.zoom_save_timer.isActive())

    def test_save_failure_is_logged(self):
        app, window = self.make_window()
        app.save_config.side_effect = OSError("read-only file system")
        window.zoom_all_messages("in")

        with self.assertLogs(level="ERROR"):
            window.close()
        self.assertEqual(app.config["response_window_zoom"], window.zoom_factor)

    def test_ctrl_wheel_zooms_only_on_vertical_scroll(self):
        _, window = self.make_window()
        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Response")
        display = self.message_displays(window)[0]

        display.wheelEvent(self.ctrl_wheel_event(QtCore.QPoint(120, 0)))
        self.assertEqual(window.zoom_factor, DEFAULT_ZOOM_FACTOR)

        display.wheelEvent(self.ctrl_wheel_event(QtCore.QPoint(0, -120)))
        self.assertEqual(window.zoom_factor, DEFAULT_ZOOM_FACTOR / ZOOM_STEP)

    def test_zoom_remains_within_existing_bounds(self):
        app, window = self.make_window({"response_window_zoom": MAX_ZOOM_FACTOR})
        window.zoom_all_messages("in")
        self.assertEqual(window.zoom_factor, MAX_ZOOM_FACTOR)
        self.assertFalse(window.zoom_save_timer.isActive())

        app.config["response_window_zoom"] = MIN_ZOOM_FACTOR
        minimum_window = ResponseWindow(app, "Proofread Result")
        self.windows.append(minimum_window)
        minimum_window.zoom_all_messages("out")
        self.assertEqual(minimum_window.zoom_factor, MIN_ZOOM_FACTOR)
        self.assertFalse(minimum_window.zoom_save_timer.isActive())


if __name__ == "__main__":
    unittest.main()
