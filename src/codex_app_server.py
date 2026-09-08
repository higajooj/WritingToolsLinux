"""Small synchronous client for the official Codex App Server JSONL protocol."""

from __future__ import annotations

import json
import logging
import os
import queue
import shutil
import subprocess
import tempfile
import threading
from pathlib import Path
from typing import Callable


class CodexAppServerError(RuntimeError):
    """Base error raised by the Codex App Server integration."""


class CodexNotInstalledError(CodexAppServerError):
    """Raised when the Codex CLI cannot be found."""


class CodexProtocolError(CodexAppServerError):
    """Raised for a JSON-RPC error or invalid server response."""

    def __init__(self, message: str, code=None):
        super().__init__(message)
        self.code = code


class CodexTurnError(CodexAppServerError):
    """Raised when an App Server turn finishes unsuccessfully."""

    def __init__(self, message: str, error_info=None):
        super().__init__(message)
        self.error_info = error_info


class CodexAppServerClient:
    """Manage one ``codex app-server`` subprocess and its JSON-RPC requests."""

    CLIENT_INFO = {
        "name": "writing_tools_linux",
        "title": "Writing Tools for Linux",
        "version": "7.0",
    }

    def __init__(
        self,
        codex_home: Path,
        executable: str | None = None,
        process_factory: Callable = subprocess.Popen,
        request_timeout: float = 15.0,
        turn_timeout: float = 180.0,
    ):
        self.codex_home = Path(codex_home)
        self.executable = executable
        self.process_factory = process_factory
        self.request_timeout = request_timeout
        self.turn_timeout = turn_timeout

        self._process = None
        self._runtime_directory = None
        self._next_id = 1
        self._pending = {}
        self._pending_lock = threading.Lock()
        self._write_lock = threading.Lock()
        self._start_lock = threading.Lock()
        self._callbacks = {}
        self._callbacks_lock = threading.Lock()
        self._stopping = False

    def is_available(self) -> bool:
        """Return whether the configured Codex executable can be resolved."""
        if self.executable:
            return os.path.isfile(self.executable) and os.access(self.executable, os.X_OK)
        return shutil.which("codex") is not None

    @property
    def runtime_path(self) -> str | None:
        if self._runtime_directory is None:
            return None
        return self._runtime_directory.name

    def start(self):
        """Start and initialize App Server if it is not already running."""
        with self._start_lock:
            if self._process is not None and self._process.poll() is None:
                return

            # A previous App Server may have exited without a clean shutdown.
            # Discard its runtime directory before starting a fresh process.
            self._process = None
            self._cleanup_runtime_directory()

            executable = self.executable or shutil.which("codex")
            if not executable:
                raise CodexNotInstalledError(
                    "The Codex CLI is not installed or is not available on PATH."
                )

            self.codex_home.mkdir(parents=True, exist_ok=True, mode=0o700)
            try:
                self.codex_home.chmod(0o700)
            except OSError:
                logging.warning("Could not restrict permissions on the Codex data directory")

            self._runtime_directory = tempfile.TemporaryDirectory(
                prefix="writing-tools-codex-"
            )
            environment = os.environ.copy()
            environment["CODEX_HOME"] = str(self.codex_home)
            command = [
                executable,
                "app-server",
                "-c",
                'cli_auth_credentials_store="file"',
                "--listen",
                "stdio://",
            ]

            self._stopping = False
            try:
                self._process = self.process_factory(
                    command,
                    stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    text=True,
                    bufsize=1,
                    cwd=self._runtime_directory.name,
                    env=environment,
                )
            except OSError as exc:
                self._process = None
                self._cleanup_runtime_directory()
                raise CodexAppServerError(f"Could not start Codex App Server: {exc}") from exc

            process = self._process
            threading.Thread(target=self._read_stdout, args=(process,), daemon=True).start()
            threading.Thread(target=self._drain_stderr, args=(process,), daemon=True).start()

            try:
                self._request_started(
                    "initialize",
                    {"clientInfo": self.CLIENT_INFO},
                    timeout=self.request_timeout,
                )
                self._send({"method": "initialized", "params": {}})
            except Exception:
                self._terminate_process()
                raise

    def request(self, method: str, params=None, timeout: float | None = None):
        self.start()
        return self._request_started(method, params, timeout)

    def notify(self, method: str, params=None):
        self.start()
        message = {"method": method}
        if params is not None:
            message["params"] = params
        self._send(message)

    def on(self, method: str, callback: Callable[[dict], None]):
        """Register a notification callback and return an unsubscribe function."""
        with self._callbacks_lock:
            self._callbacks.setdefault(method, []).append(callback)

        def unsubscribe():
            with self._callbacks_lock:
                callbacks = self._callbacks.get(method, [])
                if callback in callbacks:
                    callbacks.remove(callback)

        return unsubscribe

    def run_turn(
        self,
        *,
        model: str,
        base_instructions: str,
        developer_instructions: str,
        input_text: str,
        service_tier: str | None = None,
        on_started: Callable[[str, str], None] | None = None,
    ) -> str:
        """Run one isolated turn and return its final assistant message."""
        self.start()
        thread_params = {
            "cwd": self.runtime_path,
            "approvalPolicy": "never",
            "sandbox": "read-only",
            "baseInstructions": base_instructions,
            "developerInstructions": developer_instructions,
            "ephemeral": True,
            "serviceName": "writing_tools_linux",
        }
        if model:
            thread_params["model"] = model

        thread_result = self._request_started("thread/start", thread_params)
        try:
            thread_id = thread_result["thread"]["id"]
        except (KeyError, TypeError) as exc:
            raise CodexProtocolError("Codex returned an invalid thread/start response.") from exc

        completed = threading.Event()
        completed_payload = {}

        def handle_completed(params):
            if params.get("threadId") == thread_id:
                completed_payload.update(params)
                completed.set()

        unsubscribe = self.on("turn/completed", handle_completed)
        turn_id = None
        try:
            turn_params = {
                "threadId": thread_id,
                "input": [{"type": "text", "text": input_text}],
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "readOnly", "networkAccess": False},
                "summary": "none",
            }
            if service_tier is not None:
                # An explicit "default" prevents inheriting a Fast thread tier.
                turn_params["serviceTierForTurn"] = service_tier
            turn_result = self._request_started(
                "turn/start",
                turn_params,
            )
            try:
                turn_id = turn_result["turn"]["id"]
            except (KeyError, TypeError) as exc:
                raise CodexProtocolError("Codex returned an invalid turn/start response.") from exc

            if on_started:
                on_started(thread_id, turn_id)

            if not completed.wait(self.turn_timeout):
                self._interrupt_best_effort(thread_id, turn_id)
                raise CodexTurnError("The Codex request timed out.")

            turn = completed_payload.get("turn") or {}
            status = turn.get("status")
            if status != "completed":
                error = turn.get("error") or {}
                raise CodexTurnError(
                    error.get("message") or f"The Codex turn ended with status: {status}.",
                    error.get("codexErrorInfo"),
                )

            messages = [
                item for item in (turn.get("items") or [])
                if item.get("type") == "agentMessage" and item.get("text")
            ]
            finals = [item for item in messages if item.get("phase") == "final_answer"]
            selected = finals[-1] if finals else (messages[-1] if messages else None)
            if not selected:
                raise CodexProtocolError("Codex completed without returning any text.")
            return selected["text"].strip()
        finally:
            unsubscribe()

    def interrupt(self, thread_id: str, turn_id: str):
        self.request(
            "turn/interrupt",
            {"threadId": thread_id, "turnId": turn_id},
            timeout=5.0,
        )

    def shutdown(self):
        """Stop App Server and fail any outstanding callers."""
        with self._start_lock:
            self._stopping = True
            self._terminate_process()

    def _request_started(self, method: str, params=None, timeout: float | None = None):
        request_id = self._allocate_id()
        response_queue = queue.Queue(maxsize=1)
        with self._pending_lock:
            self._pending[request_id] = response_queue

        message = {"method": method, "id": request_id}
        if params is not None:
            message["params"] = params

        try:
            self._send(message)
            try:
                response = response_queue.get(
                    timeout=self.request_timeout if timeout is None else timeout
                )
            except queue.Empty as exc:
                raise CodexProtocolError(f"Codex timed out while handling {method}.") from exc
            if isinstance(response, Exception):
                raise response
            if "error" in response:
                error = response.get("error") or {}
                raise CodexProtocolError(
                    error.get("message") or f"Codex rejected {method}.",
                    error.get("code"),
                )
            if "result" not in response:
                raise CodexProtocolError(f"Codex returned an invalid response for {method}.")
            return response["result"]
        finally:
            with self._pending_lock:
                self._pending.pop(request_id, None)

    def _allocate_id(self):
        with self._pending_lock:
            request_id = self._next_id
            self._next_id += 1
            return request_id

    def _send(self, message: dict):
        process = self._process
        if process is None or process.poll() is not None or process.stdin is None:
            raise CodexAppServerError("Codex App Server is not running.")
        payload = json.dumps(message, ensure_ascii=False)
        try:
            with self._write_lock:
                process.stdin.write(payload + "\n")
                process.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise CodexAppServerError("The Codex App Server connection closed.") from exc

    def _read_stdout(self, process=None):
        process = process or self._process
        if process is None or process.stdout is None:
            return
        for line in process.stdout:
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                logging.warning("Ignoring malformed output from Codex App Server")
                continue

            if "id" in message and "method" in message:
                self._reject_server_request(message)
                continue

            if "id" in message:
                with self._pending_lock:
                    response_queue = self._pending.get(message["id"])
                if response_queue is not None:
                    try:
                        response_queue.put_nowait(message)
                    except queue.Full:
                        pass
                continue

            method = message.get("method")
            if not method:
                continue
            with self._callbacks_lock:
                callbacks = list(self._callbacks.get(method, []))
            for callback in callbacks:
                try:
                    callback(message.get("params") or {})
                except Exception:
                    logging.exception("Codex notification callback failed")

        # An old reader must never fail requests belonging to a replacement
        # process that has already started.
        if not self._stopping and self._process is process:
            self._fail_pending(CodexAppServerError("Codex App Server exited unexpectedly."))

    def _drain_stderr(self, process=None):
        process = process or self._process
        if process is None or process.stderr is None:
            return
        # App Server can print login URLs or other sensitive diagnostics. Drain
        # the pipe to prevent blocking, but never copy its contents into our log.
        for _line in process.stderr:
            pass

    def _reject_server_request(self, message):
        """Deny any tool or approval request the writing-only client receives."""
        method = message.get("method")
        safe_results = {
            "item/commandExecution/requestApproval": {"decision": "decline"},
            "item/fileChange/requestApproval": {"decision": "decline"},
            "item/tool/requestUserInput": {"answers": {}},
            "mcpServer/elicitation/request": {"action": "decline"},
            "item/tool/call": {"contentItems": [], "success": False},
        }
        if method in safe_results:
            response = {"id": message["id"], "result": safe_results[method]}
        else:
            response = {
                "id": message["id"],
                "error": {
                    "code": -32601,
                    "message": "Writing Tools does not provide interactive tools.",
                },
            }
        try:
            self._send(response)
        except CodexAppServerError:
            pass

    def _fail_pending(self, error: Exception):
        with self._pending_lock:
            pending = list(self._pending.values())
        for response_queue in pending:
            try:
                response_queue.put_nowait(error)
            except queue.Full:
                pass

    def _interrupt_best_effort(self, thread_id: str, turn_id: str):
        try:
            self._request_started(
                "turn/interrupt",
                {"threadId": thread_id, "turnId": turn_id},
                timeout=5.0,
            )
        except CodexAppServerError:
            pass

    def _terminate_process(self):
        process = self._process
        self._process = None
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
        self._fail_pending(CodexAppServerError("Codex App Server stopped."))
        self._cleanup_runtime_directory()

    def _cleanup_runtime_directory(self):
        if self._runtime_directory is not None:
            self._runtime_directory.cleanup()
            self._runtime_directory = None
