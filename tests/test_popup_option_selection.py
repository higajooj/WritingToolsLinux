"""Popup option selection and per-run instruction behavior."""

import copy
from pathlib import Path
import sys
import threading
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

# Qt must be pinned to the offscreen platform before PySide6 is imported.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import qt_offscreen  # noqa: F401,E402

from PySide6 import QtCore, QtGui, QtWidgets
from PySide6.QtCore import Signal, Slot
from PySide6.QtTest import QTest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from WritingToolApp import WritingToolApp
from ui.CustomPopupWindow import CustomPopupWindow
from ui.ResponseWindow import ResponseWindow


OPTIONS = {
    "Proofread": {
        "prefix": "Proofread this:\n\n",
        "instruction": "Correct the text.",
        "icon": "icons/magnifying-glass",
        "open_in_window": False,
    },
    "Summary": {
        "prefix": "Summarize this:\n\n",
        "instruction": "Summarize the text.",
        "icon": "icons/summary",
        "open_in_window": True,
    },
    "Custom": {
        "prefix": "Make this change:\n\n",
        "instruction": "Apply the described change.",
        "icon": "icons/summary",
        "open_in_window": False,
    },
}


class PopupOptionSelectionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def setUp(self):
        self.app = SimpleNamespace(
            config={},
            process_option=Mock(),
            apply_options=Mock(),
            exit_app=Mock(),
        )
        self.options_patch = patch.object(
            CustomPopupWindow,
            "load_options",
            side_effect=lambda: copy.deepcopy(OPTIONS),
        )
        self.load_options_mock = self.options_patch.start()
        self.window = CustomPopupWindow(self.app)
        self.window.show()
        QtWidgets.QApplication.processEvents()

    def tearDown(self):
        self.window.close()
        self.window.deleteLater()
        self.options_patch.stop()
        QtWidgets.QApplication.processEvents()

    def button(self, key):
        return next(button for button in self.window.button_widgets if button.key == key)

    def test_focus_timer_does_not_outlive_the_popup(self):
        """A popup torn down before its focus timer fires must not touch the
        deleted QLineEdit.  PySide reports such a failure through sys.excepthook
        rather than raising into the test, so the hook is what we assert on."""
        window = CustomPopupWindow(self.app)
        window.show()
        window.close()
        window.deleteLater()
        QtWidgets.QApplication.processEvents()  # runs the pending delete

        errors = []
        with patch.object(sys, "excepthook", lambda _t, exc, _tb: errors.append(exc)):
            QTest.qWait(400)  # past the 250ms focus timer

        self.assertEqual(errors, [])

    def test_click_selects_option_without_dispatching_or_closing(self):
        self.button("Proofread").click()

        self.app.process_option.assert_not_called()
        self.assertTrue(self.window.isVisible())
        self.assertEqual(self.window.selected_option, "Proofread")
        self.assertTrue(self.button("Proofread").property("selected"))
        self.assertEqual(
            self.window.custom_input.placeholderText(),
            "Add instructions (optional)...",
        )

    def test_enter_runs_selected_option_with_optional_instructions(self):
        self.button("Proofread").click()
        self.window.custom_input.setText("Use British English")
        self.window.custom_input.setFocus()
        QTest.keyClick(self.window.custom_input, QtCore.Qt.Key_Return)

        self.app.process_option.assert_called_once_with(
            "Proofread",
            "Use British English",
        )
        self.assertFalse(self.window.isVisible())

    def test_empty_input_runs_selected_option_normally(self):
        self.button("Proofread").click()
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with("Proofread", None)

    def test_text_without_selection_keeps_custom_action(self):
        self.window.custom_input.setText("Turn this into verse")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with(
            "Custom",
            "Turn this into verse",
        )

    def test_switching_options_retains_text_and_moves_selection(self):
        self.button("Proofread").click()
        self.window.custom_input.setText("Keep the headings")
        self.button("Summary").click()

        self.assertEqual(self.window.selected_option, "Summary")
        self.assertEqual(self.window.custom_input.text(), "Keep the headings")
        self.assertFalse(self.button("Proofread").property("selected"))
        self.assertTrue(self.button("Summary").property("selected"))

    def test_empty_input_without_selection_does_nothing(self):
        self.window.send_button.click()

        self.app.process_option.assert_not_called()
        self.assertTrue(self.window.isVisible())

    def test_clicking_selected_option_again_returns_to_custom(self):
        self.button("Proofread").click()
        self.button("Proofread").click()

        self.assertIsNone(self.window.selected_option)
        self.assertFalse(self.button("Proofread").property("selected"))
        self.assertEqual(
            self.window.custom_input.placeholderText(),
            "Describe your change...",
        )

        self.window.custom_input.setText("Turn this into verse")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with(
            "Custom",
            "Turn this into verse",
        )

    def test_empty_option_name_is_still_a_selection(self):
        self.window.on_generic_instruction("")
        self.window.send_button.click()

        self.app.process_option.assert_called_once_with("", None)

    def test_entering_edit_mode_clears_selection(self):
        self.button("Proofread").click()
        self.window.toggle_edit_mode()

        self.assertIsNone(self.window.selected_option)
        self.assertFalse(self.button("Proofread").property("selected"))

    def test_leaving_edit_mode_keeps_the_popup_and_app_running(self):
        self.window.toggle_edit_mode()
        self.window.toggle_edit_mode()

        self.assertTrue(self.window.isVisible())
        self.assertFalse(self.window.edit_mode)
        self.assertTrue(self.window.input_area.isVisible())
        self.app.exit_app.assert_not_called()

        self.button("Proofread").click()
        self.assertEqual(self.window.selected_option, "Proofread")

    def test_committed_change_refreshes_live_editor_buttons(self):
        self.window.toggle_edit_mode()
        old_buttons = list(self.window.button_widgets)
        updated = copy.deepcopy(OPTIONS)
        updated["New Action"] = {
            "prefix": "Change this:\n\n",
            "instruction": "Make the change.",
            "icon": "icons/custom",
            "open_in_window": False,
        }

        with patch.object(self.window, "save_options") as save_options:
            self.assertTrue(self.window._commit_options(updated))

        save_options.assert_called_once_with(updated)
        self.app.apply_options.assert_called_once_with(updated)
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Proofread", "Summary", "New Action"],
        )
        self.assertTrue(self.window.edit_mode)
        self.assertTrue(self.window.isVisible())
        self.assertTrue(all(button.icon_container for button in self.window.button_widgets))
        self.assertTrue(all(button.isHidden() for button in old_buttons))
        self.app.exit_app.assert_not_called()

    def test_failed_save_keeps_live_options_and_widgets_unchanged(self):
        self.window.toggle_edit_mode()
        updated = copy.deepcopy(OPTIONS)
        updated.pop("Proofread")

        with (
            patch.object(self.window, "save_options", side_effect=OSError("read-only")),
            patch.object(self.window, "_show_options_error") as show_error,
        ):
            self.assertFalse(self.window._commit_options(updated))

        self.app.apply_options.assert_not_called()
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Proofread", "Summary"],
        )
        show_error.assert_called_once()

    def test_adding_button_applies_immediately_and_stays_in_editor(self):
        self.window.toggle_edit_mode()
        dialog = Mock()
        dialog.exec_.return_value = True
        dialog.get_button_data.return_value = {
            "name": "New Action",
            "prefix": "Change this:\n\n",
            "instruction": "Make the change.",
            "icon": "icons/custom",
            "open_in_window": False,
        }

        with (
            patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
            patch.object(self.window, "save_options") as save_options,
        ):
            self.window.add_new_button_clicked()

        saved = save_options.call_args.args[0]
        self.assertIn("New Action", saved)
        self.assertEqual(self.app.apply_options.call_args.args[0], saved)
        self.assertIn("New Action", [button.key for button in self.window.button_widgets])
        self.assertTrue(self.window.edit_mode)
        self.assertTrue(self.window.isVisible())
        self.app.exit_app.assert_not_called()

    def test_editing_button_applies_rename_in_place(self):
        self.window.toggle_edit_mode()
        dialog = Mock()
        dialog.exec_.return_value = True
        dialog.get_button_data.return_value = {
            "name": "Polish",
            "prefix": "Polish this:\n\n",
            "instruction": "Polish the text.",
            "icon": "icons/custom",
            "open_in_window": False,
        }

        with (
            patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
            patch.object(self.window, "save_options") as save_options,
        ):
            self.window.edit_button_clicked(self.button("Proofread"))

        saved = save_options.call_args.args[0]
        self.assertEqual(list(saved), ["Polish", "Summary", "Custom"])
        self.assertEqual(saved["Polish"]["instruction"], "Polish the text.")
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Polish", "Summary"],
        )
        self.assertTrue(self.window.edit_mode)
        self.app.exit_app.assert_not_called()

    def edit_dialog(self, name, exec_results=(True, False)):
        dialog = Mock()
        dialog.exec_.side_effect = list(exec_results)
        dialog.get_button_data.return_value = {
            "name": name,
            "prefix": "Change this:\n\n",
            "instruction": "Make the change.",
            "icon": "icons/custom",
            "open_in_window": False,
        }
        return dialog

    def test_add_rejects_names_that_would_replace_other_options(self):
        self.window.toggle_edit_mode()

        for name in ("Summary", "Custom", ""):
            with self.subTest(name=name):
                dialog = self.edit_dialog(name)
                with (
                    patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
                    patch.object(QtWidgets.QMessageBox, "warning") as warning,
                    patch.object(self.window, "save_options") as save_options,
                ):
                    self.window.add_new_button_clicked()

                warning.assert_called_once()
                save_options.assert_not_called()
                # The dialog reopened with the entries so the name can be fixed.
                self.assertEqual(dialog.exec_.call_count, 2)

        self.app.apply_options.assert_not_called()

    def test_edit_rejects_another_buttons_name_but_keeps_its_own(self):
        self.window.toggle_edit_mode()

        dialog = self.edit_dialog("Summary")
        with (
            patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
            patch.object(QtWidgets.QMessageBox, "warning") as warning,
            patch.object(self.window, "save_options") as save_options,
        ):
            self.window.edit_button_clicked(self.button("Proofread"))
        warning.assert_called_once()
        save_options.assert_not_called()

        dialog = self.edit_dialog("Proofread", exec_results=(True,))
        with (
            patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
            patch.object(QtWidgets.QMessageBox, "warning") as warning,
            patch.object(self.window, "save_options") as save_options,
        ):
            self.window.edit_button_clicked(self.button("Proofread"))
        warning.assert_not_called()
        saved = save_options.call_args.args[0]
        self.assertEqual(list(saved), ["Proofread", "Summary", "Custom"])
        self.assertEqual(saved["Proofread"]["instruction"], "Make the change.")

    def test_failed_save_reopens_add_dialog_with_entries(self):
        self.window.toggle_edit_mode()
        dialog = self.edit_dialog("New Action", exec_results=(True, True))

        with (
            patch("ui.CustomPopupWindow.ButtonEditDialog", return_value=dialog),
            patch.object(self.window, "save_options", side_effect=[OSError("disk full"), None]),
            patch.object(self.window, "_show_options_error") as show_error,
        ):
            self.window.add_new_button_clicked()

        show_error.assert_called_once()
        self.assertEqual(dialog.exec_.call_count, 2)
        self.app.apply_options.assert_called_once()
        self.assertIn("New Action", [button.key for button in self.window.button_widgets])

    def test_popup_fits_its_contents_after_edits(self):
        many = copy.deepcopy(OPTIONS)
        for index in range(6):
            many[f"Action {index}"] = copy.deepcopy(OPTIONS["Proofread"])

        def commit(options):
            with patch.object(self.window, "save_options"):
                self.window._commit_options(copy.deepcopy(options))
            # Checked before events run, so there is no grow-then-shrink flicker.
            self.assertEqual(self.window.size(), self.window.sizeHint())
            return self.window.height()

        normal_height = self.window.height()
        self.window.toggle_edit_mode()
        self.assertEqual(self.window.size(), self.window.sizeHint())

        many_height = commit(many)
        few_height = commit(OPTIONS)
        none_height = commit({"Custom": OPTIONS["Custom"]})
        self.assertGreater(many_height, few_height)
        self.assertGreater(few_height, none_height)

        commit(many)
        commit(OPTIONS)
        self.window.toggle_edit_mode()
        self.assertEqual(self.window.size(), self.window.sizeHint())
        self.assertEqual(self.window.height(), normal_height)

    def test_button_added_in_edit_mode_is_selectable_after_leaving(self):
        self.window.toggle_edit_mode()
        updated = copy.deepcopy(OPTIONS)
        updated["New Action"] = copy.deepcopy(OPTIONS["Proofread"])

        with patch.object(self.window, "save_options"):
            self.window._commit_options(updated)
        self.window.toggle_edit_mode()

        self.button("New Action").click()
        self.assertEqual(self.window.selected_option, "New Action")

    def use_buttons(self, names):
        options = {
            name: copy.deepcopy(OPTIONS["Proofread"])
            for name in names
        }
        options["Custom"] = copy.deepcopy(OPTIONS["Custom"])
        self.load_options_mock.side_effect = lambda: copy.deepcopy(options)
        self.window.build_buttons_list(options)
        self.window.rebuild_grid_layout()
        QtWidgets.QApplication.processEvents()
        return options

    def order(self):
        return [button.key for button in self.window.button_widgets]

    def index_mime(self, source_index):
        mime = QtCore.QMimeData()
        mime.setData("application/x-button-index", str(source_index).encode())
        return mime

    def drag_mime(self, source_key):
        return self.index_mime(self.window.button_widgets.index(self.button(source_key)))

    def drag_enter(self, target, mime, side="before"):
        x = 5 if side == "before" else target.width() - 5
        return QtGui.QDragEnterEvent(
            QtCore.QPoint(x, 5),
            QtCore.Qt.MoveAction,
            mime,
            QtCore.Qt.LeftButton,
            QtCore.Qt.NoModifier,
        )

    def drop(self, source_key, target_key, side="before", source_index=None):
        """Drop on `target_key`'s chosen half; `source_index` forges the payload."""
        target = self.button(target_key)
        x = 5 if side == "before" else target.width() - 5
        mime = (
            self.drag_mime(source_key) if source_index is None
            else self.index_mime(source_index)
        )
        event = QtGui.QDropEvent(
            QtCore.QPointF(x, 5),
            QtCore.Qt.MoveAction,
            mime,
            QtCore.Qt.LeftButton,
            QtCore.Qt.NoModifier,
        )
        target.dropEvent(event)
        return event

    def test_failed_reorder_restores_original_order(self):
        self.use_buttons(["A", "B", "C", "D", "E"])
        failures = {
            "save": patch.object(self.window, "save_options", side_effect=OSError("read-only")),
            "load": patch.object(CustomPopupWindow, "load_options", side_effect=ValueError("bad JSON")),
        }
        self.window.toggle_edit_mode()

        for failure, failing_patch in failures.items():
            with self.subTest(failure=failure):
                with failing_patch, patch.object(self.window, "_show_options_error") as show_error:
                    self.drop("E", "B", side="before")

                show_error.assert_called_once()
                self.assertEqual(
                    [button.key for button in self.window.button_widgets],
                    ["A", "B", "C", "D", "E"],
                )

        self.app.apply_options.assert_not_called()

    def test_reorder_by_drop_saves_new_order(self):
        self.window.toggle_edit_mode()

        with patch.object(self.window, "save_options") as save_options:
            self.drop("Summary", "Proofread")

        self.assertEqual(list(save_options.call_args.args[0]), ["Custom", "Summary", "Proofread"])
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Summary", "Proofread"],
        )

    def test_reorder_inserts_and_shifts_buttons_instead_of_swapping(self):
        self.use_buttons(["A", "B", "C", "D", "E"])
        self.window.toggle_edit_mode()

        with patch.object(self.window, "save_options") as save_options:
            self.drop("E", "B", side="before")

        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["A", "E", "B", "C", "D"],
        )
        self.assertEqual(
            list(save_options.call_args.args[0]),
            ["Custom", "A", "E", "B", "C", "D"],
        )

    def test_reorder_forward_adjusts_the_insertion_index(self):
        self.use_buttons(["A", "B", "C", "D", "E"])
        self.window.toggle_edit_mode()

        with patch.object(self.window, "save_options"):
            self.drop("A", "D", side="after")

        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["B", "C", "D", "A", "E"],
        )

    def test_reorder_supports_first_last_and_no_op_gaps(self):
        self.use_buttons(["A", "B", "C", "D", "E"])
        self.window.toggle_edit_mode()

        with patch.object(self.window, "save_options") as save_options:
            # To the front, then back to the end.
            self.drop("E", "A", side="before")
            self.assertEqual(self.order(), ["E", "A", "B", "C", "D"])
            self.drop("E", "D", side="after")
            self.assertEqual(self.order(), ["A", "B", "C", "D", "E"])

            # Dropping on yourself, and into the gap you already fill, are
            # both no-ops that must not cost a save.
            self.drop("B", "B", side="before")
            self.assertEqual(self.order(), ["A", "B", "C", "D", "E"])
            self.drop("B", "A", side="after")
            self.assertEqual(self.order(), ["A", "B", "C", "D", "E"])

        self.assertEqual(save_options.call_count, 2)

    def test_drag_indicator_tracks_pointer_half_and_clears(self):
        self.window.toggle_edit_mode()
        target = self.button("Proofread")
        other = self.button("Summary")
        mime = self.drag_mime("Summary")

        enter = QtGui.QDragEnterEvent(
            QtCore.QPoint(5, 5),
            QtCore.Qt.MoveAction,
            mime,
            QtCore.Qt.LeftButton,
            QtCore.Qt.NoModifier,
        )
        target.dragEnterEvent(enter)
        self.assertIs(self.window.drop_indicator_button, target)
        self.assertEqual(self.window.drop_indicator_side, "before")
        self.assertIn("border-left", target.styleSheet())

        move = QtGui.QDragMoveEvent(
            QtCore.QPoint(target.width() - 5, 5),
            QtCore.Qt.MoveAction,
            mime,
            QtCore.Qt.LeftButton,
            QtCore.Qt.NoModifier,
        )
        target.dragMoveEvent(move)
        self.assertEqual(self.window.drop_indicator_side, "after")
        self.assertIn("border-right", target.styleSheet())
        self.assertNotIn("border-left: 3px solid", target.styleSheet())

        target.dragLeaveEvent(QtGui.QDragLeaveEvent())
        self.assertIsNone(self.window.drop_indicator_button)
        self.assertEqual(target.styleSheet(), target.base_style)

        self.window.show_drop_indicator(other, "before")
        self.window.rebuild_grid_layout()
        self.assertIsNone(self.window.drop_indicator_button)
        self.assertEqual(other.styleSheet(), other.base_style)

        self.window.show_drop_indicator(target, "after")
        self.window.toggle_edit_mode()
        self.assertIsNone(self.window.drop_indicator_button)
        self.assertEqual(target.styleSheet(), target.base_style)

    def test_indicator_handoff_survives_either_enter_leave_order(self):
        self.window.toggle_edit_mode()
        first = self.button("Proofread")
        second = self.button("Summary")
        mime = self.drag_mime("Summary")

        # Crossing from one button to the next, Qt may deliver dragLeave on the
        # button being left either before or after dragEnter on the one being
        # entered. Exactly one indicator must survive, on the entered button.
        first.dragEnterEvent(self.drag_enter(first, mime))
        first.dragLeaveEvent(QtGui.QDragLeaveEvent())
        second.dragEnterEvent(self.drag_enter(second, mime))

        self.assertIs(self.window.drop_indicator_button, second)
        self.assertEqual(first.styleSheet(), first.base_style)

        # Same crossing, opposite delivery order: the late dragLeave must not
        # take down the indicator the new button already claimed.
        first.dragEnterEvent(self.drag_enter(first, mime))
        second.dragLeaveEvent(QtGui.QDragLeaveEvent())

        self.assertIs(self.window.drop_indicator_button, first)
        self.assertEqual(second.styleSheet(), second.base_style)
        self.assertIn("border-left: 3px solid", first.styleSheet())

    def test_stale_drag_payload_is_rejected(self):
        self.use_buttons(["A", "B", "C"])
        self.window.toggle_edit_mode()
        self.app.apply_options.reset_mock()

        with patch.object(self.window, "save_options") as save_options:
            for source_index in (3, 99, -1):
                with self.subTest(source_index=source_index):
                    self.window.show_drop_indicator(self.button("B"), "before")
                    event = self.drop("A", "B", source_index=source_index)

                    self.assertFalse(event.isAccepted())
                    self.assertEqual(self.order(), ["A", "B", "C"])
                    self.assertIsNone(self.window.drop_indicator_button)

        save_options.assert_not_called()
        self.app.apply_options.assert_not_called()

    def test_drop_clears_indicator(self):
        self.window.toggle_edit_mode()
        target = self.button("Proofread")
        self.window.show_drop_indicator(target, "before")

        with patch.object(self.window, "save_options"):
            self.drop("Summary", "Proofread", side="before")

        self.assertIsNone(self.window.drop_indicator_button)
        self.assertEqual(target.styleSheet(), target.base_style)

    def test_cancelled_drag_clears_indicator(self):
        self.window.toggle_edit_mode()
        source = self.button("Summary")
        target = self.button("Proofread")
        self.window.show_drop_indicator(target, "before")
        source.drag_start_position = QtCore.QPoint(1, 1)
        mouse_event = Mock()
        mouse_event.buttons.return_value = QtCore.Qt.LeftButton
        mouse_event.pos.return_value = QtCore.QPoint(30, 1)
        drag = Mock()
        drag.exec_.return_value = QtCore.Qt.IgnoreAction

        with patch("ui.CustomPopupWindow.QtGui.QDrag", return_value=drag):
            source.mouseMoveEvent(mouse_event)

        self.assertIsNone(self.window.drop_indicator_button)
        self.assertEqual(target.styleSheet(), target.base_style)

    def test_deleting_button_applies_immediately(self):
        self.window.toggle_edit_mode()

        with (
            patch.object(QtWidgets.QMessageBox, "exec_", return_value=QtWidgets.QMessageBox.Yes),
            patch.object(self.window, "save_options") as save_options,
        ):
            self.window.delete_button_clicked(self.button("Proofread"))

        saved = save_options.call_args.args[0]
        self.assertNotIn("Proofread", saved)
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Summary"],
        )
        self.assertTrue(self.window.edit_mode)
        self.assertTrue(self.window.isVisible())
        self.app.exit_app.assert_not_called()

    def test_reset_applies_defaults_immediately(self):
        self.window.toggle_edit_mode()
        defaults = {
            "Default Action": {
                "prefix": "Change this:\n\n",
                "instruction": "Use the default.",
                "icon": "icons/custom",
                "open_in_window": False,
            },
            "Custom": copy.deepcopy(OPTIONS["Custom"]),
        }

        with (
            patch.object(QtWidgets.QMessageBox, "exec_", return_value=QtWidgets.QMessageBox.Yes),
            patch("ui.CustomPopupWindow.reset_options_file", return_value=defaults),
        ):
            self.window.on_reset_clicked()

        self.app.apply_options.assert_called_once_with(defaults)
        self.assertEqual(
            [button.key for button in self.window.button_widgets],
            ["Default Action"],
        )
        self.assertTrue(self.window.edit_mode)
        self.assertTrue(self.window.isVisible())
        self.app.exit_app.assert_not_called()

    def test_reordering_updates_live_options_without_rebuilding_buttons(self):
        self.window.toggle_edit_mode()
        original_widgets = list(self.window.button_widgets)
        self.window.button_widgets.reverse()

        with patch.object(self.window, "save_options") as save_options:
            self.assertTrue(self.window.update_json_from_grid())

        saved = save_options.call_args.args[0]
        self.assertEqual(list(saved), ["Custom", "Summary", "Proofread"])
        self.app.apply_options.assert_called_once_with(saved)
        self.assertEqual(self.window.button_widgets, list(reversed(original_widgets)))
        self.assertTrue(self.window.edit_mode)
        self.app.exit_app.assert_not_called()


class _OptionsApplyHost:
    _build_shortcut_map = WritingToolApp._build_shortcut_map
    apply_options = WritingToolApp.apply_options

    def __init__(self, options):
        self.config = {"shortcut": "ctrl+space"}
        self.options = options
        self.register_hotkey = Mock()


class LiveOptionApplicationTests(unittest.TestCase):
    def test_only_hotkey_definition_changes_refresh_portal_bindings(self):
        original = copy.deepcopy(OPTIONS)
        host = _OptionsApplyHost(original)

        instruction_change = copy.deepcopy(original)
        instruction_change["Proofread"]["instruction"] = "Use perfect grammar."
        host.apply_options(instruction_change)
        host.register_hotkey.assert_not_called()

        reordered = {
            "Summary": instruction_change["Summary"],
            "Proofread": instruction_change["Proofread"],
            "Custom": instruction_change["Custom"],
        }
        host.apply_options(reordered)
        host.register_hotkey.assert_not_called()

        with_hotkey = copy.deepcopy(reordered)
        with_hotkey["Proofread"]["hotkey"] = "CTRL+J"
        host.apply_options(with_hotkey)
        host.register_hotkey.assert_called_once_with()

        same_hotkey_new_prompt = copy.deepcopy(with_hotkey)
        same_hotkey_new_prompt["Proofread"]["instruction"] = "Fix every typo."
        host.apply_options(same_hotkey_new_prompt)
        host.register_hotkey.assert_called_once_with()

        renamed = copy.deepcopy(same_hotkey_new_prompt)
        renamed["Polish"] = renamed.pop("Proofread")
        host.apply_options(renamed)
        self.assertEqual(host.register_hotkey.call_count, 2)

    def test_refresh_follows_the_shortcuts_that_would_be_bound(self):
        options = copy.deepcopy(OPTIONS)
        options["Proofread"]["hotkey"] = "ctrl+j"
        host = _OptionsApplyHost(options)

        def change(button, hotkey):
            updated = copy.deepcopy(host.options)
            updated[button]["hotkey"] = hotkey
            host.apply_options(updated)

        # The portal keeps key case, so a case-only change must rebind.
        change("Proofread", "ctrl+J")
        self.assertEqual(host.register_hotkey.call_count, 1)

        # Triggers that are skipped at registration bind nothing new.
        change("Summary", "ctrl")
        change("Summary", "shift")
        change("Summary", "CTRL+SPACE")
        change("Summary", "ctrl+J")
        self.assertEqual(host.register_hotkey.call_count, 1)

        # Freeing the conflicting trigger lets Summary's hotkey bind.
        change("Proofread", "ctrl+k")
        self.assertEqual(host.register_hotkey.call_count, 2)
        self.assertIn("button:Summary", host._build_shortcut_map(host.options))


class OptionPromptTests(unittest.TestCase):
    def setUp(self):
        self.app = SimpleNamespace(options=OPTIONS)

    def build_prompt(self, option, selected_text, additional_instructions=None):
        return WritingToolApp._build_option_prompt(
            self.app,
            option,
            selected_text,
            additional_instructions,
        )

    def test_empty_addition_preserves_existing_named_option_prompt(self):
        self.assertEqual(
            self.build_prompt("Proofread", "Original text"),
            "Proofread this:\n\nOriginal text",
        )

    def test_named_option_includes_additional_instructions(self):
        self.assertEqual(
            self.build_prompt("Proofread", "Original text", "Keep slang"),
            "Proofread this:\n\nAdditional instructions: Keep slang"
            "\n\nText:\nOriginal text",
        )

    def test_custom_prompt_keeps_existing_shape(self):
        self.assertEqual(
            self.build_prompt("Custom", "Original text", "Make it rhyme"),
            "Make this change:\n\nDescribed change: Make it rhyme"
            "\n\nText: Original text",
        )

    def build_system_instruction(self, option, additional_instructions=None):
        return WritingToolApp._build_system_instruction(
            self.app,
            option,
            additional_instructions,
        )

    def test_system_instruction_unchanged_without_additions(self):
        self.assertEqual(
            self.build_system_instruction("Proofread", "   "),
            "Correct the text.",
        )

    def test_system_instruction_gives_additions_precedence(self):
        instruction = self.build_system_instruction("Proofread", " Use a warmer tone ")

        self.assertTrue(instruction.startswith("Correct the text.\n\n"))
        self.assertIn("override the rules above", instruction)
        self.assertTrue(instruction.endswith("\nUse a warmer tone"))

    def test_custom_system_instruction_is_unchanged(self):
        self.assertEqual(
            self.build_system_instruction("Custom", "Make it rhyme"),
            "Apply the described change.",
        )


class _ResponseWindowStub(QtCore.QObject):
    def __init__(self):
        super().__init__()
        self.chat_history = []
        self.text = None

    @Slot(str)
    def set_text(self, text):
        self.text = text


class _OptionWorkerHost(QtCore.QObject):
    """The slice of WritingToolApp that `process_option_thread` touches."""

    show_message_signal = Signal(str, str)

    _setup_response_window = WritingToolApp._setup_response_window
    _build_option_prompt = WritingToolApp._build_option_prompt
    _build_system_instruction = WritingToolApp._build_system_instruction
    process_option_thread = WritingToolApp.process_option_thread


class ProcessOptionThreadTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def test_window_option_seeds_history_with_the_prompt_sent_to_provider(self):
        host = _OptionWorkerHost()
        host.options = OPTIONS
        host.input_backend = Mock()
        host.current_text_holder = SimpleNamespace(ready=threading.Event(), text="Long text")
        host.current_text_holder.ready.set()
        window = _ResponseWindowStub()
        host.show_response_window = Mock(return_value=window)
        host.current_provider = Mock()
        host.current_provider.get_response.return_value = "Short summary"

        worker = threading.Thread(
            target=host.process_option_thread,
            args=("Summary", "Use bullets"),
        )
        worker.start()
        while worker.is_alive():
            QtWidgets.QApplication.processEvents()
        worker.join()
        QtWidgets.QApplication.processEvents()

        prompt = "Summarize this:\n\nAdditional instructions: Use bullets\n\nText:\nLong text"
        host.show_response_window.assert_called_once_with("Summary", "Long text")
        self.assertEqual(window.chat_history, [{"role": "user", "content": prompt}])
        system_instruction, sent_prompt = host.current_provider.get_response.call_args.args
        self.assertEqual(sent_prompt, prompt)
        self.assertIn("Use bullets", system_instruction)
        self.assertEqual(window.text, "Short summary")


class ResponseHistoryTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QtWidgets.QApplication.instance() or QtWidgets.QApplication([])

    def test_followups_wait_for_initial_response(self):
        class App(QtCore.QObject):
            followup_response_signal = Signal(str)

        app = App()
        app.config = {}
        window = ResponseWindow(app, "Summary Result")
        self.addCleanup(window.deleteLater)

        self.assertFalse(window.input_field.isEnabled())

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.set_text("Short summary")

        self.assertTrue(window.input_field.isEnabled())

    def test_initial_response_keeps_preseeded_request(self):
        request = "Summarize this:\n\nAdditional instructions: Use bullets\n\nText:\nLong text"
        response = SimpleNamespace(
            chat_history=[{"role": "user", "content": request}],
            option="Summary",
            selected_text="Long text",
            stop_thinking_animation=Mock(),
            chat_area=Mock(),
            _add_message=Mock(),
            app=SimpleNamespace(config={}),
            _adjust_window_height=Mock(),
        )

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            ResponseWindow.set_text(response, "Short summary")

        self.assertEqual(
            response.chat_history,
            [
                {"role": "user", "content": request},
                {"role": "assistant", "content": "Short summary"},
            ],
        )

    def test_window_displays_original_clipboard_text_before_response(self):
        class App(QtCore.QObject):
            followup_response_signal = Signal(str)

        app = App()
        app.config = {}
        window = ResponseWindow(app, "Summary Result")
        self.addCleanup(window.deleteLater)

        with patch("ui.ResponseWindow.QtCore.QTimer.singleShot"):
            window.display_original_text("Long text")

            first_display = window.chat_area.layout.itemAt(0).widget().layout().itemAt(0).widget()
            self.assertTrue(first_display.is_user_message)
            self.assertIn("Long text", first_display.toPlainText())

            window.set_text("Short summary")

        second_display = window.chat_area.layout.itemAt(1).widget().layout().itemAt(0).widget()
        self.assertFalse(second_display.is_user_message)
        self.assertIn("Short summary", second_display.toPlainText())


if __name__ == "__main__":
    unittest.main()
