import asyncio
import http.client
import json
import signal
import socket
import threading
import time

import pytest

from thermite import App, RouteError


def free_port(family=socket.AF_INET, host="127.0.0.1"):
    with socket.socket(family) as listener:
        listener.bind((host, 0))
        return listener.getsockname()[1]


def fetch(port, path, host="127.0.0.1"):
    connection = http.client.HTTPConnection(host, port, timeout=5)
    try:
        connection.request("GET", path)
        response = connection.getresponse()
        return response.status, response.read()
    finally:
        connection.close()


def fetch_when_ready(port, path, host="127.0.0.1"):
    deadline = time.monotonic() + 5
    while True:
        try:
            return fetch(port, path, host)
        except ConnectionError:
            if time.monotonic() > deadline:
                raise
            time.sleep(0.01)


# app.run has to be on the main thread, so the client is the one that moves
def serve_while(app, client, stop=signal.SIGINT, host="127.0.0.1"):
    port = free_port(
        socket.AF_INET6 if ":" in host else socket.AF_INET,
        host,
    )
    answers = []

    def run_client():
        try:
            answers.append(client(port))
        finally:
            signal.raise_signal(stop)

    thread = threading.Thread(target=run_client)
    thread.start()
    try:
        app.run(host=host, port=port)
    finally:
        thread.join()
    return answers[0]


def small_app():
    app = App()

    @app.get("/plain")
    def plain():
        return "plain"

    @app.get("/async")
    async def asynchronous():
        asyncio.get_running_loop()
        return threading.current_thread() is threading.main_thread()

    return app


def test_a_running_app_answers_over_a_socket():
    def client(port):
        return (
            fetch_when_ready(port, "/plain"),
            fetch(port, "/missing")[0],
            json.loads(fetch(port, "/openapi.json")[1])["paths"].keys() >= {"/plain"},
        )

    assert serve_while(small_app(), client) == ((200, b'"plain"'), 404, True)


def test_async_handlers_run_on_the_loop_of_the_main_thread():
    assert serve_while(small_app(), lambda port: fetch_when_ready(port, "/async")) == (
        200,
        b"true",
    )


@pytest.mark.parametrize("stop", [signal.SIGINT, signal.SIGTERM])
def test_a_signal_stops_the_server_and_restores_the_handlers(stop):
    before = signal.getsignal(stop)

    serve_while(small_app(), lambda port: fetch_when_ready(port, "/plain"), stop)

    assert signal.getsignal(stop) is before


def test_a_request_in_flight_finishes_during_shutdown():
    started = threading.Event()
    app = App()

    @app.get("/slow")
    async def slow():
        started.set()
        await asyncio.sleep(0.3)
        return "finished"

    answers = []

    # leaving the client sends the signal, with the request still waiting
    def client(port):
        thread = threading.Thread(
            target=lambda: answers.append(fetch_when_ready(port, "/slow"))
        )
        thread.start()
        assert started.wait(timeout=5)
        return thread

    thread = serve_while(app, client)
    thread.join()

    assert answers == [(200, b'"finished"')]


def test_a_taken_port_is_an_os_error():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]

        with pytest.raises(OSError, match=f"127.0.0.1:{port}"):
            small_app().run(port=port)


def test_an_ipv6_host_is_bracketed():
    if not socket.has_ipv6:
        pytest.skip("no IPv6")
    try:
        with socket.socket(socket.AF_INET6) as listener:
            listener.bind(("::1", 0))
    except OSError:
        pytest.skip("no IPv6 loopback")

    answer = serve_while(
        small_app(),
        lambda port: fetch_when_ready(port, "/plain", "::1"),
        host="::1",
    )

    assert answer == (200, b'"plain"')


def test_a_route_shadowing_the_document_stops_the_server_from_starting():
    app = App()
    app.get("/openapi.json")(lambda: None)

    with pytest.raises(RouteError, match="/openapi.json"):
        app.run()


def test_routes_can_be_added_again_once_the_server_stopped():
    app = small_app()

    serve_while(app, lambda port: fetch_when_ready(port, "/plain"))
    app.get("/later")(lambda: None)

    assert "/later" in app.openapi()["paths"]
