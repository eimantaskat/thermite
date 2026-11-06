import pytest

from thermite._thermite import CoreApp, Field, Schema


def test_a_default_that_is_not_json_text_is_a_value_error():
    with pytest.raises(ValueError, match="expected"):
        Field.with_default("limit", Schema.integer(), "not json")


def test_an_invalid_method_is_a_value_error():
    with pytest.raises(ValueError, match="invalid HTTP method"):
        CoreApp().add_route("NOT A METHOD", "/items", "items", [], None, None, None)


def client_for(route):
    core = CoreApp()
    core.add_route("GET", "/items", "items", [], None, None, route)
    return core.test_server()


def test_a_route_whose_start_raises_answers_500_with_that_error():
    class Route:
        def start(self, arguments, body, respond):
            raise ValueError("boom")

    status, _, content, error = client_for(Route()).send("GET", "/items", [], b"")

    assert (status, content) == (500, b"Internal Server Error")
    assert isinstance(error, ValueError)


def test_a_route_that_never_responds_answers_500():
    class Route:
        def start(self, arguments, body, respond):
            pass

    status, _, content, error = client_for(Route()).send("GET", "/items", [], b"")

    assert (status, content, error) == (500, b"Internal Server Error", None)


def test_only_the_first_answer_of_a_route_counts():
    class Route:
        def start(self, arguments, body, respond):
            respond.succeed(b"1")
            respond.succeed(b"2")

    assert client_for(Route()).send("GET", "/items", [], b"")[2] == b"1"
