"""Tests for the Codex App Server JSONL transport and turn orchestration."""

import io
import json
import os
from pathlib import Path
import queue
import sys
import tempfile
import threading
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from codex_app_server import (
    CodexAppServerClient,
    CodexProtocolError,
    CodexTurnError,
)


class _BlockingLines:
    def __init__(self):
        self.lines = queue.Queue()

    def __iter__(self):
        return self

    def __next__(self):
        line = self.lines.get(timeout=2)
        if line is None:
            raise StopIteration
        return line

    def put(self, message):
        self.lines.put(json.dumps(message) + "\n")

    def close(self):
        self.lines.put(None)


class _FakeStdin:
    def __init__(self, process):
        self.process = process

    def write(self, payload):
        message = json.loads(payload)
        self.process.messages.append(message)
        if "id" not in message or "method" not in message:
            return
        if message["method"] == "initialize":
            result = {"userAgent": "fake-codex"}
        elif message["method"] == "account/read":
            result = {"account": None, "requiresOpenaiAuth": True}
        else:
            result = {}
        self.process.stdout.put({"id": message["id"], "result": result})

    def flush(self):
        pass


class _FakeProcess:
    def __init__(self):
        self.messages = []
        self.stdout = _BlockingLines()
        self.stderr = io.StringIO("")
        self.stdin = _FakeStdin(self)
        self.returncode = None

    def poll(self):
        return self.returncode

    def terminate(self):
        self.returncode = 0
        self.stdout.close()

    def kill(self):
        self.terminate()

    def wait(self, timeout=None):
        self.returncode = 0
        return self.returncode


class _ScriptedTurnClient(CodexAppServerClient):
    def __init__(self, completed_turn):
        super().__init__(Path("/unused"), turn_timeout=0.1)
        self.completed_turn = completed_turn
        self.calls = []

    @property
    def runtime_path(self):
        return "/private/runtime"

    def start(self):
        pass

    def _request_started(self, method, params=None, timeout=None):
        self.calls.append((method, params))
        if method == "thread/start":
            return {"thread": {"id": "thread-1"}}
        if method == "turn/start":
            for callback in self._callbacks.get("turn/completed", []):
                callback({"threadId": "thread-1", "turn": self.completed_turn})
            return {"turn": {"id": "turn-1"}}
        return {}


class CodexAppServerClientTests(unittest.TestCase):
    def test_starts_with_isolated_home_and_initializes_protocol(self):
        created = {}

        def process_factory(command, **kwargs):
            created["command"] = command
            created["kwargs"] = kwargs
            created["process"] = _FakeProcess()
            return created["process"]

        with tempfile.TemporaryDirectory() as directory:
            codex_home = Path(directory) / "writing-tools-codex"
            client = CodexAppServerClient(
                codex_home,
                executable="/usr/bin/fake-codex",
                process_factory=process_factory,
                request_timeout=1,
            )
            client.start()
            result = client.request("account/read", {"refreshToken": True})

            self.assertIsNone(result["account"])
            self.assertEqual(created["kwargs"]["env"]["CODEX_HOME"], str(codex_home))
            self.assertNotEqual(created["kwargs"]["cwd"], os.getcwd())
            self.assertEqual(created["command"], [
                "/usr/bin/fake-codex",
                "app-server",
                "-c",
                'cli_auth_credentials_store="file"',
                "--listen",
                "stdio://",
            ])
            messages = created["process"].messages
            self.assertEqual(messages[0]["method"], "initialize")
            self.assertEqual(messages[1], {"method": "initialized", "params": {}})
            self.assertEqual(messages[2]["method"], "account/read")
            client.shutdown()

    def test_turn_is_ephemeral_read_only_and_returns_final_answer(self):
        client = _ScriptedTurnClient({
            "id": "turn-1",
            "status": "completed",
            "items": [
                {"type": "agentMessage", "phase": "commentary", "text": "Working"},
                {"type": "agentMessage", "phase": "final_answer", "text": "  Edited.  "},
            ],
        })
        started = []

        result = client.run_turn(
            model="gpt-test",
            base_instructions="Only edit text.",
            developer_instructions="Proofread.",
            input_text="Some text",
            on_started=lambda thread_id, turn_id: started.append((thread_id, turn_id)),
        )

        self.assertEqual(result, "Edited.")
        self.assertEqual(started, [("thread-1", "turn-1")])
        thread_params = client.calls[0][1]
        self.assertTrue(thread_params["ephemeral"])
        self.assertEqual(thread_params["sandbox"], "read-only")
        self.assertEqual(thread_params["approvalPolicy"], "never")
        self.assertEqual(thread_params["cwd"], "/private/runtime")
        turn_params = client.calls[1][1]
        self.assertEqual(
            turn_params["sandboxPolicy"],
            {"type": "readOnly", "networkAccess": False},
        )
        self.assertEqual(turn_params["input"], [{"type": "text", "text": "Some text"}])

    def test_unsuccessful_turn_exposes_structured_error(self):
        client = _ScriptedTurnClient({
            "id": "turn-1",
            "status": "failed",
            "items": [],
            "error": {
                "message": "Limit reached",
                "codexErrorInfo": "usageLimitExceeded",
            },
        })

        with self.assertRaises(CodexTurnError) as raised:
            client.run_turn(
                model="",
                base_instructions="",
                developer_instructions="",
                input_text="Text",
            )
        self.assertEqual(raised.exception.error_info, "usageLimitExceeded")

    def test_protocol_error_preserves_error_code(self):
        client = CodexAppServerClient(Path("/unused"))
        client._process = _FakeProcess()

        def send_error(message):
            response_queue = client._pending[message["id"]]
            response_queue.put({
                "id": message["id"],
                "error": {"code": -32601, "message": "Method not found"},
            })

        client._send = send_error
        with self.assertRaises(CodexProtocolError) as raised:
            client._request_started("new/method")
        self.assertEqual(raised.exception.code, -32601)

    def test_server_tool_requests_are_explicitly_denied(self):
        client = CodexAppServerClient(Path("/unused"))
        sent = []
        client._send = sent.append

        client._reject_server_request({
            "id": "server-1",
            "method": "item/commandExecution/requestApproval",
            "params": {},
        })
        client._reject_server_request({
            "id": "server-2",
            "method": "attestation/generate",
            "params": {},
        })

        self.assertEqual(sent[0], {
            "id": "server-1",
            "result": {"decision": "decline"},
        })
        self.assertEqual(sent[1]["error"]["code"], -32601)


if __name__ == "__main__":
    unittest.main()
