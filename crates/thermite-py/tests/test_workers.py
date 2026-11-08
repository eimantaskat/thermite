import logging
import multiprocessing
import os
import signal
import socket
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from multiprocessing.context import SpawnProcess

import pytest

import workers_app
import workers_broken_app
from test_server import fetch, fetch_when_ready, free_port
from thermite import App, RouteError
from thermite._thermite import CoreApp
from thermite._workers import _reachable, _stop_when_told, _wake_accept


def answering_pids(port, wanted):
    seen = set()
    deadline = time.monotonic() + 30
    with ThreadPoolExecutor(16) as pool:
        while len(seen) < wanted:
            assert time.monotonic() < deadline, seen
            answers = pool.map(lambda _: fetch(port, "/pid"), range(16))
            seen.update(int(body) for _, body in answers)
    return seen


# app.run has to be on the main thread, so the client is the one that moves
def serve_while(client, stop=signal.SIGINT, workers=2):
    port = free_port()
    answers = []

    def run_client():
        try:
            answers.append(client(port))
        finally:
            signal.raise_signal(stop)

    thread = threading.Thread(target=run_client)
    thread.start()
    try:
        workers_app.app.run(port=port, workers=workers)
    finally:
        thread.join()
    return answers[0]


def run_script(tmp_path, source, *arguments):
    script = tmp_path / "script.py"
    script.write_text(source)
    return subprocess.run(
        [sys.executable, str(script), *arguments],
        capture_output=True,
        text=True,
        timeout=60,
    )


def test_workers_share_one_port_across_processes():
    def client(port):
        fetch_when_ready(port, "/blocking", timeout=30)
        return answering_pids(port, 2)

    pids = serve_while(client)

    assert len(pids) == 2
    assert os.getpid() not in pids


@pytest.mark.parametrize("stop", [signal.SIGINT, signal.SIGTERM])
def test_a_signal_stops_every_worker_and_closes_the_port(stop):
    before = signal.getsignal(stop)
    ports = []

    def client(port):
        ports.append(port)
        return fetch_when_ready(port, "/blocking", timeout=30)[0]

    assert serve_while(client, stop) == 200
    assert signal.getsignal(stop) is before
    with pytest.raises(ConnectionError):
        fetch(ports[0], "/blocking")


def test_requests_in_flight_finish_when_stopping():
    answers = []
    started = threading.Event()

    def request(port):
        fetch_when_ready(port, "/blocking", timeout=30)
        started.set()
        answers.append(fetch(port, "/slow?seconds=0.5"))

    def client(port):
        thread = threading.Thread(target=request, args=(port,))
        thread.start()
        assert started.wait(40)
        time.sleep(0.2)
        return thread

    thread = serve_while(client)
    thread.join()

    assert answers == [(200, b'"finished"')]


def test_a_second_signal_skips_waiting_for_requests():
    finished = []

    def request(port):
        try:
            fetch(port, "/slow?seconds=30")
        except OSError as error:
            finished.append(error)

    def client(port):
        fetch_when_ready(port, "/blocking", timeout=30)
        thread = threading.Thread(target=request, args=(port,))
        thread.start()
        time.sleep(0.2)
        signal.raise_signal(signal.SIGINT)
        time.sleep(0.5)
        return thread

    started = time.monotonic()
    thread = serve_while(client, workers=2)

    assert time.monotonic() - started < 25
    thread.join()
    assert len(finished) == 1


def test_a_worker_that_dies_is_replaced(caplog):
    def client(port):
        fetch_when_ready(port, "/blocking", timeout=30)
        first, second = answering_pids(port, 2)
        os.kill(first, signal.SIGTERM if os.name == "nt" else signal.SIGKILL)
        deadline = time.monotonic() + 30
        while True:
            assert time.monotonic() < deadline
            try:
                pids = answering_pids(port, 2)
            except ConnectionError:
                continue
            if first not in pids:
                return pids, second

    with caplog.at_level(logging.ERROR, logger="thermite.error"):
        pids, second = serve_while(client)

    assert second in pids
    assert "restarting" in caplog.text


def test_a_worker_that_cannot_start_stops_the_others_and_raises():
    with pytest.raises(RuntimeError, match="before serving"):
        workers_broken_app.app.run(port=free_port(), workers=2)


def test_fewer_than_one_worker_is_a_value_error():
    with pytest.raises(ValueError, match="at least 1, got 0"):
        workers_app.app.run(workers=0)


def test_an_app_that_is_not_a_global_is_refused():
    with pytest.raises(RuntimeError, match="global"):
        App().run(workers=2)


def test_a_taken_port_is_an_os_error():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]

        with pytest.raises(OSError, match=str(port)):
            workers_app.app.run(port=port, workers=2)


def test_a_route_shadowing_the_document_stops_it_before_any_worker(monkeypatch):
    app = App()
    app.get("/openapi.json")(lambda: None)
    monkeypatch.setattr(workers_app, "shadowing", app, raising=False)

    with pytest.raises(RouteError, match="/openapi.json"):
        app.run(workers=2)


def test_a_main_module_without_a_guard_is_told_about_it(tmp_path):
    source = """
import sys
from thermite import App

app = App()
app.run(port=int(sys.argv[1]), workers=2)
"""
    finished = run_script(tmp_path, source, str(free_port()))

    assert finished.returncode == 1
    assert 'if __name__ == "__main__"' in finished.stderr
    assert "before serving" in finished.stderr


def test_being_reached_inside_a_worker_is_refused(monkeypatch):
    monkeypatch.setattr(multiprocessing.current_process(), "name", "SpawnProcess-1")

    with pytest.raises(RuntimeError, match="__main__"):
        workers_app.app.run(workers=2)


def test_failing_to_start_a_worker_leaves_none_behind(monkeypatch):
    started = []
    original = SpawnProcess.start

    def start_once(process):
        if started:
            raise OSError("out of processes")
        original(process)
        started.append(process)

    monkeypatch.setattr(SpawnProcess, "start", start_once)

    with pytest.raises(OSError, match="out of processes"):
        workers_app.app.run(port=free_port(), workers=2)

    assert started[0].exitcode == 0


def test_a_main_module_with_a_guard_serves_until_stopped(tmp_path):
    source = """
import http.client, signal, sys, threading, time
from thermite import App

app = App()

@app.get("/plain")
def plain():
    return "plain"

def client():
    deadline = time.monotonic() + 30
    while True:
        try:
            connection = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
            connection.request("GET", "/plain")
            print(connection.getresponse().read())
            connection.close()
            break
        except ConnectionError:
            assert time.monotonic() < deadline
            time.sleep(0.05)
    signal.raise_signal(signal.SIGINT)

if __name__ == "__main__":
    port = int(sys.argv[1])
    threading.Thread(target=client).start()
    app.run(port=port, workers=2)
    print("stopped")
"""
    finished = run_script(tmp_path, source, str(free_port()))

    assert finished.returncode == 0, finished.stderr
    assert finished.stdout.split() == ["b'\"plain\"'", "stopped"]


def test_each_stop_and_the_end_of_the_pipe_request_a_shutdown():
    class Server:
        requests = 0

        def request_shutdown(self):
            self.requests += 1

    ours, theirs = multiprocessing.Pipe()
    ours.send_bytes(b"stop")
    ours.close()
    server = Server()
    stopping = threading.Event()

    _stop_when_told(theirs, server, stopping)

    assert server.requests == 2
    assert stopping.is_set()


def test_stopping_wakes_an_accept_and_ends_once_the_port_is_closed():
    stopping = threading.Event()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        address = listener.getsockname()
        waking = threading.Thread(target=_wake_accept, args=(address, stopping))
        waking.start()
        listener.settimeout(0.3)
        with pytest.raises(TimeoutError):
            listener.accept()

        stopping.set()
        listener.settimeout(5)
        listener.accept()[0].close()

    waking.join(5)
    assert not waking.is_alive()


@pytest.mark.parametrize(
    ("bound", "reached"),
    [
        (("0.0.0.0", 80), ("127.0.0.1", 80)),
        (("::", 80, 0, 0), ("::1", 80)),
        (("127.0.0.2", 80), ("127.0.0.2", 80)),
    ],
)
def test_a_socket_on_every_address_is_reached_through_loopback(bound, reached):
    assert _reachable(bound) == reached


def test_a_worker_on_every_address_still_stops():
    port = free_port()

    def client():
        answer = fetch_when_ready(port, "/blocking", timeout=30)
        signal.raise_signal(signal.SIGINT)
        return answer

    with ThreadPoolExecutor(1) as pool:
        answer = pool.submit(client)
        workers_app.app.run(host="0.0.0.0", port=port, workers=2)
        assert answer.result()[0] == 200


def test_serving_something_other_than_a_socket_is_a_type_error():
    server = CoreApp().server(None)

    with pytest.raises(TypeError, match="socket"):
        server.serve_socket(1)


def test_serving_a_closed_socket_is_a_value_error():
    server = CoreApp().server(None)
    closed = socket.socket()
    closed.close()

    with pytest.raises(ValueError, match="closed"):
        server.serve_socket(closed)
