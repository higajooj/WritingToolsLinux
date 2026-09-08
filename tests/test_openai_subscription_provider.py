"""Tests for browser-based ChatGPT subscription provider behavior."""

import json
from pathlib import Path
import sys
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

from PySide6 import QtCore
from PySide6.QtWidgets import QApplication, QVBoxLayout

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from aiprovider import OpenAISubscriptionProvider, _CodexSettingsWidget
from codex_app_server import CodexTurnError
from ui.SettingsWindow import SettingsWindow


class _FakeCodexClient:
    def __init__(self, available=True):
        self.available = available
        self.account = None
        self.models_pages = [[
            {"model": "gpt-first", "displayName": "GPT First", "hidden": False},
            {"model": "hidden", "displayName": "Hidden", "hidden": True},
        ]]
        # "repeat" hands back a cursor that never advances; "unique" always
        # offers another page. Both must terminate.
        self.model_cursor_mode = None
        self.requests = []
        self.handlers = {}
        self.turns = []
        self.interrupts = []
        self.shutdown_called = False
        self.turn_error = None

    def is_available(self):
        return self.available

    def start(self):
        if not self.available:
            raise AssertionError("Unavailable fake should not be started")

    def on(self, method, callback):
        self.handlers.setdefault(method, []).append(callback)
        return lambda: self.handlers[method].remove(callback)

    def emit(self, method, params):
        for callback in list(self.handlers.get(method, [])):
            callback(params)

    def request(self, method, params=None, timeout=None):
        self.requests.append((method, params))
        if method == "account/read":
            return {"account": self.account, "requiresOpenaiAuth": True}
        if method == "account/login/start":
            return {"loginId": "login-1", "authUrl": "https://example.test/login"}
        if method == "account/login/cancel":
            return {"status": "cancelled"}
        if method == "account/logout":
            self.account = None
            return {}
        if method == "model/list":
            if self.model_cursor_mode == "repeat":
                return {"data": [{"model": "gpt-loop"}], "nextCursor": "same"}
            if self.model_cursor_mode == "unique":
                calls = len([c for c in self.requests if c[0] == "model/list"])
                return {"data": [{"model": f"gpt-{calls}"}], "nextCursor": f"c{calls}"}
            page = 1 if params and params.get("cursor") else 0
            data = self.models_pages[page]
            next_cursor = "next" if page + 1 < len(self.models_pages) else None
            return {"data": data, "nextCursor": next_cursor}
        return {}

    def run_turn(self, **kwargs):
        self.turns.append(kwargs)
        kwargs["on_started"]("thread-1", "turn-1")
        if self.turn_error:
            raise self.turn_error
        return "Edited response"

    def interrupt(self, thread_id, turn_id):
        self.interrupts.append((thread_id, turn_id))

    def shutdown(self):
        self.shutdown_called = True


class OpenAISubscriptionProviderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.app = SimpleNamespace(
            config={"providers": {}},
            output_ready_signal=Mock(),
            show_message_signal=Mock(),
            save_config=Mock(),
        )
        self.client = _FakeCodexClient()
        self.provider = OpenAISubscriptionProvider(self.app, client=self.client)

    def wait_for(self, predicate, timeout=1):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            QApplication.processEvents()
            if predicate():
                return
            time.sleep(0.01)
        self.fail("Timed out waiting for asynchronous provider operation")

    def test_browser_login_refreshes_account_and_dynamic_models(self):
        with patch("aiprovider.webbrowser.open", return_value=True) as open_browser:
            self.provider.login_async()
            self.wait_for(lambda: self.provider.pending_login_id == "login-1")

        open_browser.assert_called_once_with("https://example.test/login")
        login_call = next(call for call in self.client.requests if call[0] == "account/login/start")
        self.assertEqual(login_call[1], {
            "type": "chatgpt",
            "useHostedLoginSuccessPage": True,
            "appBrand": "chatgpt",
        })

        self.client.account = {
            "type": "chatgpt",
            "email": "writer@example.test",
            "planType": "plus",
        }
        self.client.emit("account/login/completed", {"loginId": "login-1", "success": True})
        self.wait_for(lambda: self.provider.auth_state == "signed_in")
        self.assertIn("writer@example.test", self.provider.auth_message)
        self.assertEqual([model["model"] for model in self.provider.models], ["gpt-first"])

    def test_model_list_paginates_and_filters_hidden_models(self):
        self.client.models_pages = [
            [{"model": "gpt-one", "hidden": False}],
            [
                {"model": "gpt-two", "hidden": False},
                {"model": "hidden", "hidden": True},
            ],
        ]
        self.client.account = {"type": "chatgpt", "email": "writer@example.test"}
        self.provider._refresh_account()

        self.assertEqual(
            [model["model"] for model in self.provider.models],
            ["gpt-one", "gpt-two"],
        )
        model_calls = [call for call in self.client.requests if call[0] == "model/list"]
        self.assertEqual(model_calls[0][1], {"includeHidden": False})
        self.assertEqual(model_calls[1][1], {"includeHidden": False, "cursor": "next"})

    def test_pending_browser_login_can_be_cancelled(self):
        self.provider.pending_login_id = "login-1"
        self.provider.auth_state = "signing_in"

        self.provider.cancel_login_async()
        self.wait_for(lambda: self.provider.pending_login_id is None)

        self.assertIn(
            ("account/login/cancel", {"loginId": "login-1"}),
            self.client.requests,
        )
        self.assertEqual(self.provider.auth_state, "signed_out")

    def test_generation_passes_writing_instructions_history_and_selected_model(self):
        self.client.account = {"type": "chatgpt", "email": "writer@example.test"}
        self.provider.model = "gpt-first"
        self.provider.models = self.client.models_pages[0][:1]
        messages = [
            {"role": "system", "content": "Keep it concise."},
            {"role": "user", "content": "Rewrite this."},
            {"role": "assistant", "content": "First draft."},
            {"role": "user", "content": "Make it warmer."},
        ]

        result = self.provider.get_response("Use plain English.", messages, return_response=True)

        self.assertEqual(result, "Edited response")
        turn = self.client.turns[0]
        self.assertEqual(turn["model"], "gpt-first")
        self.assertEqual(turn["service_tier"], "default")
        self.assertIn("Use plain English.", turn["developer_instructions"])
        self.assertIn("Keep it concise.", turn["developer_instructions"])
        history = json.loads(turn["input_text"].split("Conversation JSON:\n", 1)[1])
        self.assertEqual(history, messages[1:])
        self.app.output_ready_signal.emit.assert_not_called()

    def test_only_model_and_speed_choices_are_saved(self):
        self.provider.model = "gpt-first"
        self.provider.service_tier = "priority"
        self.provider.account = {
            "type": "chatgpt",
            "email": "private@example.test",
            "accessToken": "secret",
        }
        self.provider.save_config()

        self.assertEqual(
            self.app.config["providers"][self.provider.provider_name],
            {"model": "gpt-first", "service_tier": "priority"},
        )

    def test_saved_speed_applies_to_automatic_and_explicit_models(self):
        self.client.account = {"type": "chatgpt"}
        for model in ("", "gpt-first"):
            for tier in ("priority", "default"):
                with self.subTest(model=model, tier=tier):
                    self.provider.load_config({"model": model, "service_tier": tier})
                    self.assertEqual(
                        self.provider.get_response("Proofread.", "Text", return_response=True),
                        "Edited response",
                    )
                    self.assertEqual(self.client.turns[-1]["model"], model)
                    self.assertEqual(self.client.turns[-1]["service_tier"], tier)

    def test_missing_or_invalid_speed_resets_to_standard(self):
        for config in ({}, {"service_tier": None}, {"service_tier": "invalid"}):
            with self.subTest(config=config):
                self.provider.service_tier = "priority"
                self.provider.load_config(config)
                self.assertEqual(self.provider.service_tier, "default")

    def test_speed_survives_settings_save_restart_and_widget_recreation(self):
        self.client.account = {"type": "chatgpt"}
        window = self._settings_window()
        self.wait_for(lambda: self.provider.auth_state == "signed_in")
        for tier in ("priority", "default"):
            with self.subTest(tier=tier):
                dropdown = self.provider.settings_widget.speed_setting.dropdown
                dropdown.setCurrentIndex(dropdown.findData(tier))
                window.save_settings()
                saved = self.app.config["providers"][self.provider.provider_name]
                self.assertEqual(saved["service_tier"], tier)
                restarted = OpenAISubscriptionProvider(self.app, client=self.client)
                restarted.load_config(saved)
                self.assertEqual(restarted.service_tier, tier)
                self.provider.settings_widget.deleteLater()
                QApplication.sendPostedEvents(None, QtCore.QEvent.Type.DeferredDelete)
                self.assertIsNone(self.provider.settings_widget)
                window = SettingsWindow(self.app, providers_only=True)
                self.addCleanup(window.deleteLater)
                self.assertEqual(self.provider.settings_widget.selected_service_tier(), tier)
                self.wait_for(lambda: self.provider.auth_state == "signed_in")

    def test_fast_failure_does_not_retry_at_standard_speed(self):
        self.client.account = {"type": "chatgpt"}
        self.client.turn_error = CodexTurnError("Unsupported service tier")
        self.provider.load_config({"service_tier": "priority"})
        self.assertEqual(self.provider.get_response("Proofread.", "Text"), "")
        self.assertEqual(len(self.client.turns), 1)
        self.assertEqual(self.client.turns[0]["service_tier"], "priority")
        self.app.show_message_signal.emit.assert_called_once()

    def test_unavailable_saved_model_falls_back_to_automatic(self):
        self.provider.model = "retired-model"
        self.provider.models = [{"model": "gpt-first"}]
        self.assertEqual(self.provider._resolve_model(), "")
        self.assertEqual(self.provider.model, "")

    def test_usage_limit_error_is_actionable(self):
        self.client.account = {"type": "chatgpt"}
        self.client.turn_error = CodexTurnError(
            "Limit reached", "usageLimitExceeded"
        )

        self.assertEqual(self.provider.get_response("Proofread.", "Text"), "")
        self.app.show_message_signal.emit.assert_called_once_with(
            "ChatGPT Usage Limit Reached",
            "Your ChatGPT plan has reached a usage limit. Please try again later.",
        )

    def test_settings_keep_saved_model_until_authoritative_list_arrives(self):
        self.client.available = False
        self.provider.model = "saved-model"
        widget = _CodexSettingsWidget(self.provider)
        self.addCleanup(widget.deleteLater)

        widget.apply_models({"models": [], "authoritative": False})
        self.assertEqual(self.provider.model, "saved-model")
        widget.apply_models({"models": [{"model": "gpt-first"}], "authoritative": True})
        self.assertEqual(self.provider.model, "")
        self.assertTrue(widget.model_warning.isVisibleTo(widget))

    def test_shutdown_stops_app_server(self):
        self.provider.before_load()
        self.assertTrue(self.client.shutdown_called)

    def _settings_window(self):
        self.app.providers = [self.provider]
        self.app.config = {
            "provider": self.provider.provider_name,
            "providers": {self.provider.provider_name: {"model": ""}},
        }
        self.app.create_tray_icon = Mock()
        self.app.register_hotkey = Mock()
        window = SettingsWindow(self.app, providers_only=True)
        self.addCleanup(window.deleteLater)
        return window

    def test_settings_cannot_activate_provider_without_the_codex_cli(self):
        self.client.available = False
        window = self._settings_window()

        with patch("ui.SettingsWindow.QtWidgets.QMessageBox.warning") as warning:
            window.save_settings()

        warning.assert_called_once()
        self.assertIn("Install", warning.call_args[0][2])
        self.app.save_config.assert_not_called()

    def test_settings_cannot_activate_provider_before_sign_in(self):
        window = self._settings_window()
        self.wait_for(lambda: self.provider.auth_state == "signed_out")

        with patch("ui.SettingsWindow.QtWidgets.QMessageBox.warning") as warning:
            window.save_settings()

        warning.assert_called_once()
        self.assertIn("Sign in", warning.call_args[0][2])
        self.app.save_config.assert_not_called()

    def test_settings_wait_for_an_account_probe_that_is_still_running(self):
        self.provider.ACCOUNT_PROBE_WAIT = 3
        self.client.account = {"type": "chatgpt", "email": "writer@example.test"}
        self.provider._refresh_done.clear()
        self.provider.auth_state = "checking"

        def finish_probe():
            time.sleep(0.05)
            self.provider._refresh_account()

        threading.Thread(target=finish_probe, daemon=True).start()

        valid, message = self.provider.validate_settings()
        self.assertTrue(valid, message)

    def test_settings_stay_saveable_when_the_account_probe_fails(self):
        # Offline or a transient Codex failure leaves the sign-in state unknown,
        # not absent. Blocking would strand the user in the Settings dialog.
        self.provider.auth_state = "error"
        self.provider.auth_message = "The Codex App Server connection closed."

        valid, message = self.provider.validate_settings()
        self.assertTrue(valid, message)

    def test_settings_ask_the_user_to_update_an_unsupported_codex(self):
        self.provider.auth_state = "unsupported"

        valid, message = self.provider.validate_settings()
        self.assertFalse(valid)
        # The message box renders plain text, so it must not be HTML-escaped.
        self.assertEqual(message, self.provider.UNSUPPORTED_MESSAGE)
        self.assertIn("Update Codex", message)

    def test_cancelling_a_turn_reports_no_error(self):
        self.client.account = {"type": "chatgpt"}

        def run_turn(**kwargs):
            kwargs["on_started"]("thread-1", "turn-1")
            # The hotkey interrupts the turn, and Codex reports the abort.
            self.provider.cancel()
            raise CodexTurnError("The Codex turn ended with status: aborted.")

        self.client.run_turn = run_turn

        self.assertEqual(self.provider.get_response("Proofread.", "Text"), "")
        # cancel() interrupts on a worker thread.
        self.wait_for(lambda: self.client.interrupts == [("thread-1", "turn-1")])
        self.app.show_message_signal.emit.assert_not_called()
        self.app.output_ready_signal.emit.assert_not_called()

    def test_cancelling_drops_a_turn_that_finished_anyway(self):
        self.client.account = {"type": "chatgpt"}

        def run_turn(**kwargs):
            kwargs["on_started"]("thread-1", "turn-1")
            # The interrupt lost the race, but the user still cancelled.
            self.provider.cancel()
            return "Edited response"

        self.client.run_turn = run_turn

        self.assertEqual(self.provider.get_response("Proofread.", "Text"), "")
        self.app.output_ready_signal.emit.assert_not_called()

    def test_a_signed_in_account_is_not_reprobed_before_every_turn(self):
        self.client.account = {"type": "chatgpt", "email": "writer@example.test"}

        self.provider.get_response("Proofread.", "Text", return_response=True)
        self.provider.get_response("Proofread.", "More text", return_response=True)

        reads = [call for call in self.client.requests if call[0] == "account/read"]
        self.assertEqual(len(reads), 1)

    def test_an_unauthorized_turn_forces_the_next_request_to_reprobe(self):
        self.client.account = {"type": "chatgpt"}
        self.provider.get_response("Proofread.", "Text", return_response=True)

        self.client.turn_error = CodexTurnError("Session expired", "unauthorized")
        self.provider.get_response("Proofread.", "Text", return_response=True)
        self.assertIsNone(self.provider.account)

        self.client.turn_error = None
        self.provider.get_response("Proofread.", "Text", return_response=True)

        reads = [call for call in self.client.requests if call[0] == "account/read"]
        self.assertEqual(len(reads), 2)

    def test_model_list_stops_when_the_cursor_never_advances(self):
        self.client.model_cursor_mode = "repeat"

        self.provider._refresh_models()

        calls = [call for call in self.client.requests if call[0] == "model/list"]
        self.assertEqual(len(calls), 2)

    def test_model_list_stops_at_the_page_cap(self):
        self.client.model_cursor_mode = "unique"

        self.provider._refresh_models()

        calls = [call for call in self.client.requests if call[0] == "model/list"]
        self.assertEqual(len(calls), self.provider.MAX_MODEL_PAGES)

    def test_settings_widget_is_forgotten_once_qt_destroys_it(self):
        layout = QVBoxLayout()
        self.addCleanup(layout.deleteLater)
        self.provider.render_settings(layout, {"model": "gpt-first"})
        widget = self.provider.settings_widget
        self.assertIsNotNone(widget)

        widget.setParent(None)
        widget.deleteLater()
        QApplication.sendPostedEvents(None, QtCore.QEvent.Type.DeferredDelete)

        self.assertIsNone(self.provider.settings_widget)


if __name__ == "__main__":
    unittest.main()
