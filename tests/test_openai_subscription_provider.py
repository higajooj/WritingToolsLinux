"""Tests for browser-based ChatGPT subscription provider behavior."""

import json
from pathlib import Path
import sys
import time
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

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
        self.assertIn("Use plain English.", turn["developer_instructions"])
        self.assertIn("Keep it concise.", turn["developer_instructions"])
        history = json.loads(turn["input_text"].split("Conversation JSON:\n", 1)[1])
        self.assertEqual(history, messages[1:])
        self.app.output_ready_signal.emit.assert_not_called()

    def test_only_model_choice_is_saved(self):
        self.provider.model = "gpt-first"
        self.provider.account = {
            "type": "chatgpt",
            "email": "private@example.test",
            "accessToken": "secret",
        }
        self.provider.save_config()

        self.assertEqual(
            self.app.config["providers"][self.provider.provider_name],
            {"model": "gpt-first"},
        )

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

    def test_settings_cannot_activate_provider_before_sign_in(self):
        self.client.available = False
        self.app.providers = [self.provider]
        self.app.config = {
            "provider": self.provider.provider_name,
            "providers": {self.provider.provider_name: {"model": ""}},
        }
        self.app.create_tray_icon = Mock()
        self.app.register_hotkey = Mock()
        window = SettingsWindow(self.app, providers_only=True)
        self.addCleanup(window.deleteLater)

        with patch("ui.SettingsWindow.QtWidgets.QMessageBox.warning") as warning:
            window.save_settings()

        warning.assert_called_once()
        self.app.save_config.assert_not_called()


if __name__ == "__main__":
    unittest.main()
