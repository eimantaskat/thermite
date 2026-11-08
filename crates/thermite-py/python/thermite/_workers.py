import asyncio
import importlib
import logging
import multiprocessing
import signal
import socket
import sys
import threading
import time
from dataclasses import dataclass
from multiprocessing.context import SpawnProcess
from types import FrameType
from typing import TYPE_CHECKING, Any

from thermite._thermite import Server

if TYPE_CHECKING:
    from thermite._app import App

# a different class on Windows and on Unix, and typeshed only knows the one it runs on
_PipeEnd = Any

_logger = logging.getLogger("thermite.error")

_BACKLOG = 1024
_SIGNALS = (signal.SIGINT, signal.SIGTERM)
# what connecting to a socket bound to every address has to use
_LOOPBACK = {"0.0.0.0": "127.0.0.1", "::": "::1"}


@dataclass
class _Worker:
    process: SpawnProcess
    # the worker takes a message, or this end closing, as a request to stop
    connection: _PipeEnd

    def request_stop(self) -> None:
        try:
            self.connection.send_bytes(b"stop")
        except OSError:
            pass  # it has just exited, which the supervisor finds out next

    def has_served(self) -> bool:
        try:
            return self.connection.poll() and bool(self.connection.recv_bytes())
        except (EOFError, OSError):
            return False


def run_workers(app: "App", host: str, port: int, count: int) -> None:
    if count < 1:
        raise ValueError(f"workers must be at least 1, got {count}")
    # the name is what a spawned process has set while it imports the main module
    if multiprocessing.current_process().name != "MainProcess":
        raise RuntimeError(
            "app.run(workers=...) was reached while a worker imported the main "
            'module; put it behind `if __name__ == "__main__":`'
        )
    location = _find_app(app)
    # what a worker would otherwise only find out on its own
    app._core.server(None)
    family, _, _, _, address = socket.getaddrinfo(host, port, type=socket.SOCK_STREAM)[
        0
    ]
    listener = socket.create_server(
        (str(address[0]), int(address[1])), family=family, backlog=_BACKLOG
    )
    with listener:
        _supervise(location, listener, count)


def _find_app(app: "App") -> tuple[str, str]:
    for module_name, module in list(sys.modules.items()):
        for name, value in list(getattr(module, "__dict__", {}).items()):
            if value is app:
                return module_name, name
    raise RuntimeError(
        "app.run(workers=...) needs the app to be a global of a module, "
        "as each worker imports it from there"
    )


def _supervise(location: tuple[str, str], listener: socket.socket, count: int) -> None:
    context = multiprocessing.get_context("spawn")

    def start_worker() -> _Worker:
        ours, theirs = context.Pipe()
        process = context.Process(
            target=_serve_worker, args=(location, listener, theirs)
        )
        process.start()
        theirs.close()
        return _Worker(process, ours)

    # a signal handler can run in the middle of starting a worker, so it only counts
    requested = 0

    def request_stop(_signal: int, _frame: FrameType | None) -> None:
        nonlocal requested
        requested += 1

    previous = {handled: signal.signal(handled, request_stop) for handled in _SIGNALS}
    workers: list[_Worker] = []
    forwarded = 0
    failure: RuntimeError | None = None
    try:
        workers.extend(start_worker() for _ in range(count))
        while workers:
            # on Windows, a signal makes waiting on processes raise; sleeping does not
            time.sleep(0.1)
            if forwarded < requested:
                forwarded = requested
                # the workers' copies keep serving; this one only kept restarts possible
                listener.close()
                for worker in workers:
                    worker.request_stop()
            for worker in [w for w in workers if not w.process.is_alive()]:
                workers.remove(worker)
                exit_code = worker.process.exitcode
                served = worker.has_served()
                worker.connection.close()
                if forwarded or failure is not None:
                    continue
                if served:
                    _logger.error("Worker exited with code %s, restarting", exit_code)
                    workers.append(start_worker())
                    continue
                failure = RuntimeError(
                    f"a worker exited with code {exit_code} before serving"
                )
                listener.close()
                for other in workers:
                    other.request_stop()
    finally:
        # a worker takes the end of its pipe as a request to stop
        for worker in workers:
            worker.connection.close()
        for worker in workers:
            worker.process.join()
        for handled, handler in previous.items():
            signal.signal(handled, handler)
    if failure is not None:
        raise failure


def _serve_worker(
    location: tuple[str, str],
    listener: socket.socket,
    parent: _PipeEnd,
) -> None:
    # a signal reaches the supervisor too, and a second request here would skip the wait
    for handled in _SIGNALS:
        signal.signal(handled, signal.SIG_IGN)
    module_name, name = location
    app: App = getattr(importlib.import_module(module_name), name)
    asyncio.run(_serve_socket(app, listener, parent))


async def _serve_socket(app: "App", listener: socket.socket, parent: _PipeEnd) -> None:
    loop = asyncio.get_running_loop()
    server = app._core.server(loop)
    try:
        parent.send_bytes(b"ready")
    except OSError:
        return  # the supervisor went away while this worker was starting
    stopping = threading.Event()
    address = _reachable(listener.getsockname())
    threading.Thread(target=_wake_accept, args=(address, stopping), daemon=True).start()
    threading.Thread(
        target=_stop_when_told, args=(parent, server, stopping), daemon=True
    ).start()
    await loop.run_in_executor(None, server.serve_socket, listener)


# a supervisor that goes away closes the pipe, so no worker is left behind
def _stop_when_told(
    parent: _PipeEnd, server: Server, stopping: threading.Event
) -> None:
    while True:
        try:
            parent.recv_bytes()
        except (EOFError, OSError):
            server.request_shutdown()
            stopping.set()
            return
        server.request_shutdown()
        stopping.set()


# on Windows, losing a race for a connection can leave a worker stuck in accept
def _wake_accept(address: tuple[str, int], stopping: threading.Event) -> None:
    stopping.wait()
    while True:
        try:
            socket.create_connection(address, timeout=1).close()
        except OSError:
            return  # refused, so every worker has stopped listening
        time.sleep(0.05)


def _reachable(address: tuple[Any, ...]) -> tuple[str, int]:
    host, port = address[:2]
    return _LOOPBACK.get(host, host), port
