"""Run with: QT_QPA_PLATFORM=offscreen python -m unittest discover -s tests"""

import json
import os
from pathlib import Path
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import httpx
from openai import OpenAI
from PySide6.QtWidgets import QApplication

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from aiprovider import OpenAICompatibleProvider
from ui.SettingsWindow import SettingsWindow


class OpenAICompatibleProviderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.qt_app = QApplication.instance() or QApplication([])

    def setUp(self):
        self.requests = []
        self.app = SimpleNamespace(
            config={},
            output_ready_signal=Mock(),
            show_message_signal=Mock(),
            save_config=Mock(),
            create_tray_icon=Mock(),
            register_hotkey=Mock(),
        )
        self.provider = OpenAICompatibleProvider(self.app)
        self.config = {
            "api_base": "http://127.0.0.1:18765/v1",
            "api_model": "gpt-5.6-sol",
            "api_organisation": "",
            "api_project": "",
        }
        factory = patch("aiprovider.OpenAI", side_effect=self.make_client)
        factory.start()
        self.addCleanup(factory.stop)

    def make_client(self, **kwargs):
        client = OpenAI(
            **kwargs,
            http_client=httpx.Client(transport=httpx.MockTransport(self.respond)),
        )
        self.addCleanup(client.close)
        return client

    def respond(self, request):
        self.requests.append(request)
        return httpx.Response(200, json={
            "id": "test-response",
            "object": "chat.completion",
            "created": 0,
            "model": self.config["api_model"],
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "  Corrected text.  "},
                "finish_reason": "stop",
            }],
        })

    def test_keyless_requests_ignore_environment_credentials(self):
        with patch.dict(os.environ, {"OPENAI_API_KEY": "environment-secret"}):
            for key_config in [{}, {"api_key": ""}, {"api_key": " \t\n"}]:
                with self.subTest(key_config=key_config):
                    self.provider.load_config(self.config | key_config)
                    result = self.provider.get_response("Proofread.", "Some text.")
                    self.assertEqual(result, "Corrected text.")
                    request = self.requests[-1]
                    self.assertNotIn("authorization", request.headers)
                    self.assertEqual(str(request.url), self.config["api_base"] + "/chat/completions")
                    body = json.loads(request.content)
                    self.assertEqual(body["model"], self.config["api_model"])
                    self.assertFalse(body["stream"])
                    self.assertNotIn("temperature", body)
                    self.assertEqual(body["messages"], [
                        {"role": "system", "content": "Proofread."},
                        {"role": "user", "content": "Some text."},
                    ])
                    self.app.output_ready_signal.emit.assert_called_with("Corrected text.")
        self.app.show_message_signal.emit.assert_not_called()

    def test_provided_key_is_sent_as_bearer(self):
        self.provider.load_config(self.config | {"api_key": "  provided-key  "})
        self.assertEqual(self.provider.get_response("Proofread.", "Text."), "Corrected text.")
        self.assertEqual(self.requests[-1].headers["authorization"], "Bearer provided-key")

    def test_keyless_follow_up_preserves_conversation(self):
        self.provider.load_config(self.config)
        messages = [
            {"role": "system", "content": "Help with writing."},
            {"role": "user", "content": "Summarize this text."},
            {"role": "assistant", "content": "A summary."},
            {"role": "user", "content": "Make it shorter."},
        ]
        result = self.provider.get_response("", messages, return_response=True)
        self.assertEqual(result, "Corrected text.")
        self.assertEqual(json.loads(self.requests[-1].content)["messages"], messages)
        self.assertNotIn("authorization", self.requests[-1].headers)
        self.app.output_ready_signal.emit.assert_not_called()

    def test_setup_and_settings_save_and_reload_empty_key(self):
        self.app.providers = [self.provider]
        for providers_only in [True, False]:
            with self.subTest(providers_only=providers_only):
                self.app.config = {
                    "provider": self.provider.provider_name,
                    "providers": {self.provider.provider_name: self.config | {"api_key": ""}},
                }
                window = SettingsWindow(self.app, providers_only=providers_only)
                self.addCleanup(window.deleteLater)
                window.save_settings()
                saved = json.loads(json.dumps(self.app.config))
                saved_provider = saved["providers"][self.provider.provider_name]
                self.assertEqual(saved_provider["api_key"], "")
                self.app.save_config.assert_called_with(self.app.config)
                restarted_provider = OpenAICompatibleProvider(self.app)
                restarted_provider.load_config(saved_provider)
                self.assertEqual(restarted_provider.get_response("Proofread.", "Text."), "Corrected text.")
                self.assertNotIn("authorization", self.requests[-1].headers)


if __name__ == "__main__":
    unittest.main()
