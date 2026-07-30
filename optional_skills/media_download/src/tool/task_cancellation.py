"""Cooperative cancellation shared by interactive media tasks and child processes."""

from __future__ import annotations

import os
import signal
import subprocess
import threading


class OperationCancelled(RuntimeError):
    """Raised when the user cancels an interactive task."""


def terminate_process(process: subprocess.Popen[object], *, force: bool = False) -> None:
    """Ask a registered child process (and its POSIX process group) to stop."""
    if process.poll() is not None:
        return
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL if force else signal.SIGTERM)
        else:  # pragma: no cover - exercised on Windows.
            process.kill() if force else process.terminate()
    except (OSError, ProcessLookupError):
        try:
            process.terminate()
        except OSError:
            pass


def force_kill_if_running(process: subprocess.Popen[object], timeout: float = 1.0) -> None:
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        terminate_process(process, force=True)


class CancellationToken:
    """Thread-safe cancellation flag that can also stop registered subprocesses."""

    def __init__(self) -> None:
        self._event = threading.Event()
        self._lock = threading.RLock()
        self._processes: set[subprocess.Popen[object]] = set()

    def is_cancelled(self) -> bool:
        return self._event.is_set()

    def wait(self, timeout: float | None = None) -> bool:
        return self._event.wait(timeout)

    def raise_if_cancelled(self) -> None:
        if self.is_cancelled():
            raise OperationCancelled("cancelled by user")

    def cancel(self) -> None:
        with self._lock:
            self._event.set()
            processes = list(self._processes)
        for process in processes:
            terminate_process(process)
            threading.Thread(
                target=force_kill_if_running,
                args=(process,),
                name="cancel-process-watchdog",
                daemon=True,
            ).start()

    def register_process(self, process: subprocess.Popen[object]) -> None:
        with self._lock:
            if self._event.is_set():
                terminate_process(process)
                threading.Thread(
                    target=force_kill_if_running,
                    args=(process,),
                    name="cancel-process-watchdog",
                    daemon=True,
                ).start()
                raise OperationCancelled("cancelled by user")
            self._processes.add(process)

    def unregister_process(self, process: subprocess.Popen[object]) -> None:
        with self._lock:
            self._processes.discard(process)
