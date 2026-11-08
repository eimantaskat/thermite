import asyncio
import subprocess
import sys

import pytest

from thermite._thermite import (
    Body,
    CoreApp,
    Field,
    FieldShape,
    ModelShape,
    Schema,
    Shape,
)


def test_a_default_that_is_not_json_text_is_a_value_error():
    with pytest.raises(ValueError, match="expected"):
        Field.with_default("limit", Schema.integer(), "not json")


def test_an_invalid_method_is_a_value_error():
    with pytest.raises(ValueError, match="invalid HTTP method"):
        CoreApp().add_route(
            "NOT A METHOD", "/items", "items", [], None, None, None, False
        )


def test_a_body_naming_a_model_that_is_not_given_is_a_value_error():
    shape = Shape.list(Shape.model(1))
    models = [ModelShape(object, [FieldShape("next", shape, False)])]

    with pytest.raises(ValueError, match="model 1 is out of range for 1 models"):
        Body("items", Schema.any(), Shape.model(0), models)


def test_a_body_that_fails_to_build_for_a_route_without_fail_answers_500():
    class Rejecting:
        def __init__(self, value):
            raise ValueError("rejected")

    class Route:
        def start(self, arguments, respond):
            respond.succeed(b"never")

    schema = Schema.model("Rejecting", [Field.required("value", Schema.integer())])
    model = ModelShape(Rejecting, [FieldShape("value", Shape.any(), False)])
    core = CoreApp()
    body = Body("item", schema, Shape.model(0), [model])
    core.add_route("POST", "/items", "items", [], body, None, Route(), False)

    status, _, _, error = core.test_server(None).send(
        "POST", "/items", [("content-type", "application/json")], b'{"value": 1}'
    )

    assert status == 500
    assert isinstance(error, AttributeError)


def client_for(route, on_event_loop=False, event_loop=None):
    core = CoreApp()
    core.add_route("GET", "/items", "items", [], None, None, route, on_event_loop)
    return core.test_server(event_loop)


def test_a_route_whose_start_raises_answers_500_with_that_error():
    class Route:
        def start(self, arguments, respond):
            raise ValueError("boom")

    status, _, content, error = client_for(Route()).send("GET", "/items", [], b"")

    assert (status, content) == (500, b"Internal Server Error")
    assert isinstance(error, ValueError)


def test_a_route_that_never_responds_answers_500():
    class Route:
        def start(self, arguments, respond):
            pass

    status, _, content, error = client_for(Route()).send("GET", "/items", [], b"")

    assert (status, content, error) == (500, b"Internal Server Error", None)


def test_only_the_first_answer_of_a_route_counts():
    class Route:
        def start(self, arguments, respond):
            respond.succeed(b"1")
            respond.succeed(b"2")

    assert client_for(Route()).send("GET", "/items", [], b"")[2] == b"1"


def test_a_route_for_an_event_loop_that_is_closed_answers_500():
    class Route:
        def start(self, arguments, respond):
            respond.succeed(b"never")

    event_loop = asyncio.new_event_loop()
    event_loop.close()
    server = client_for(Route(), on_event_loop=True, event_loop=event_loop)

    assert server.send("GET", "/items", [], b"")[:1] == (500,)
    assert server.send("GET", "/items", [], b"")[:1] == (500,)


def test_a_route_for_no_event_loop_answers_500():
    class Route:
        def start(self, arguments, respond):
            respond.succeed(b"never")

    server = client_for(Route(), on_event_loop=True)

    assert server.send("GET", "/items", [], b"")[:1] == (500,)


def test_freeing_a_test_server_does_not_wait_for_a_route_still_running():
    # a deadlock here holds the GIL, which would freeze pytest along with it
    program = """
import time
from thermite._thermite import CoreApp

class Route:
    def start(self, arguments, respond):
        respond.succeed(b"answered")
        time.sleep(0.2)

core = CoreApp()
core.add_route("GET", "/items", "items", [], None, None, Route(), False)
print(core.test_server(None).send("GET", "/items", [], b"")[2])
"""
    finished = subprocess.run(
        [sys.executable, "-c", program], capture_output=True, text=True, timeout=30
    )

    assert finished.stdout.strip() == "b'answered'"
