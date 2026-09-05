"""Tests for the Codex App Server JSONL transport and turn orchestration."""

import io
import json
import os
from pathlib import Path
import queue
import shutil
import sys
import tempfile
import threading
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from codex_app_server import (
    CodexAppServerClient,
    CodexAppServerError,
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


def _default_responder(process, message):
    """Answer any request with an empty-ish result."""
    if message["method"] == "account/read":
        result = {"account": None, "requiresOpenaiAuth": True}
    else:
        result = {}
    process.stdout.put({"id": message["id"], "result": result})


def _silent_responder(process, message):
    """Accept requests but never answer them."""


def _turn_responder(turn):
    """Drive a full thread/start + turn/start + turn/completed exchange."""

    def respond(process, message):
        if message["method"] == "thread/start":
            process.stdout.put(
                {"id": message["id"], "result": {"thread": {"id": "thread-1"}}}
            )
        elif message["method"] == "turn/start":
            process.stdout.put(
                {"id": message["id"], "result": {"turn": {"id": "turn-1"}}}
            )
            process.stdout.put({
                "method": "turn/completed",
                "params": {"threadId": "thread-1", "turn": turn},
            })
        else:
            _default_responder(process, message)

    return respond


class _FakeStdin:
    def __init__(self, process, responder=None):
        self.process = process
        self.responder = responder or _default_responder

    def write(self, payload):
        message = json.loads(payload)
        self.process.messages.append(message)
        if "id" not in message or "method" not in message:
            return
        # The handshake is always answered; everything else is up to the test.
        if message["method"] == "initialize":
            self.process.stdout.put(
                {"id": message["id"], "result": {"userAgent": "fake-codex"}}
            )
            return
        self.responder(self.process, message)

    def flush(self):
        pass


class _FakeProcess:
    def __init__(self, responder=None):
        self.messages = []
        self.stdout = _BlockingLines()
        self.stderr = io.StringIO("")
        self.stdin = _FakeStdin(self, responder)
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

    def _start_client(self, responder=None, **kwargs):
        """Start a client wired to a fake process over the real transport."""
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        process = _FakeProcess(responder=responder)
        client = CodexAppServerClient(
            Path(directory.name) / "codex",
            executable="/usr/bin/fake-codex",
            process_factory=lambda command, **kw: process,
            request_timeout=kwargs.pop("request_timeout", 2),
            turn_timeout=kwargs.pop("turn_timeout", 2),
        )
        self.addCleanup(client.shutdown)
        return client, process

    @staticmethod
    def _wait_for(predicate, timeout=2):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return True
            time.sleep(0.01)
        return False

    def test_notifications_are_dispatched_and_stop_after_unsubscribing(self):
        client, process = self._start_client()
        client.start()

        completed = []
        unsubscribe = client.on("turn/completed", completed.append)
        marker = []
        client.on("account/updated", marker.append)

        process.stdout.put(
            {"method": "turn/completed", "params": {"threadId": "thread-1"}}
        )
        self.assertTrue(self._wait_for(lambda: completed))
        self.assertEqual(completed, [{"threadId": "thread-1"}])

        unsubscribe()
        process.stdout.put(
            {"method": "turn/completed", "params": {"threadId": "thread-2"}}
        )
        # The marker is queued behind the ignored notification, so once it lands
        # we know the unsubscribed one was already processed and dropped.
        process.stdout.put({"method": "account/updated", "params": {"authMode": None}})
        self.assertTrue(self._wait_for(lambda: marker))
        self.assertEqual(len(completed), 1)

    def test_pending_requests_fail_when_the_process_exits(self):
        client, process = self._start_client(responder=_silent_responder)
        client.start()

        failures = []

        def call():
            try:
                client.request("account/read", timeout=5)
            except CodexAppServerError as exc:
                failures.append(exc)

        caller = threading.Thread(target=call)
        caller.start()
        self.assertTrue(self._wait_for(lambda: client._pending))

        process.stdout.close()
        caller.join(timeout=3)
        self.assertEqual(len(failures), 1)
        self.assertIn("exited unexpectedly", str(failures[0]))

    def test_run_turn_completes_over_the_real_request_path(self):
        turn = {
            "id": "turn-1",
            "status": "completed",
            "items": [
                {"type": "agentMessage", "phase": "commentary", "text": "Working"},
                {"type": "agentMessage", "phase": "final_answer", "text": "  Edited.  "},
            ],
        }
        client, process = self._start_client(responder=_turn_responder(turn))

        result = client.run_turn(
            model="gpt-test",
            base_instructions="Only edit text.",
            developer_instructions="Proofread.",
            input_text="Some text",
        )

        self.assertEqual(result, "Edited.")
        self.assertEqual(
            [message["method"] for message in process.messages],
            ["initialize", "initialized", "thread/start", "turn/start"],
        )
        # The completion handler must not outlive the turn it was created for.
        self.assertEqual(client._callbacks.get("turn/completed"), [])

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


@unittest.skipUnless(shutil.which("codex"), "requires the Codex CLI on PATH")
class LiveCodexAppServerTests(unittest.TestCase):
    """Check our JSONL framing against the real binary; the fakes cannot.

    Runs against a throwaway CODEX_HOME, so it never touches the user's own
    Codex login, and asks for no token refresh, so it needs no network.
    """

    def test_handshake_and_account_read_against_the_real_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            client = CodexAppServerClient(
                Path(directory) / "codex", request_timeout=60
            )
            self.addCleanup(client.shutdown)
            result = client.request("account/read", {"refreshToken": False})
            self.assertIsInstance(result, dict)


if __name__ == "__main__":
    unittest.main()
