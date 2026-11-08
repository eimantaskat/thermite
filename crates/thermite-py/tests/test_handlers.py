import asyncio
import functools
import gc
import inspect
import logging
import sys
import threading
import weakref
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass, field
from typing import Annotated, Any

import pytest

from thermite import App, Body, Cookie, Header, Query, RouteError
from thermite.testclient import TestClient


@dataclass
class Tag:
    name: str


@dataclass
class Item:
    name: str
    price: float = 1.5
    tags: list[Tag] = field(default_factory=list)
    parent: "Tag | None" = None


def client_for(handler, path="/items", method="get", **options):
    app = App()
    getattr(app, method)(path)(handler)
    return TestClient(app, **options)


def test_a_plain_handler_answers_json():
    def handler():
        return {"message": "héllo"}

    response = client_for(handler).get("/items")

    assert response.status_code == 200
    assert response.headers["content-type"] == "application/json"
    assert response.content == '{"message":"héllo"}'.encode()
    assert response.text == '{"message":"héllo"}'
    assert response.json() == {"message": "héllo"}


def test_an_async_handler_answers_json():
    async def handler():
        await asyncio.sleep(0)
        return {"message": "hello"}

    assert client_for(handler).get("/items").json() == {"message": "hello"}


def test_a_handler_returning_nothing_answers_null():
    def handler():
        pass

    response = client_for(handler).get("/items")

    assert response.status_code == 200
    assert response.content == b"null"


def test_a_callable_object_with_an_async_call_is_awaited():
    class Handler:
        async def __call__(self):
            return "awaited"

    assert client_for(Handler()).get("/items").json() == "awaited"


def test_a_partial_of_an_async_function_is_awaited():
    async def handler(name: str = "x"):
        return name

    partial = functools.partial(handler, name="a")

    assert client_for(partial).get("/items").json() == "a"


@pytest.mark.parametrize("wrap", [False, True], ids=["plain", "async"])
def test_arguments_reach_the_handler_by_name(wrap):
    def plain(
        item_id: int,
        limit: int,
        missing: str = "fallback",
        unset: str | None = None,
        tag: Annotated[list[str], Query()] = ["none"],  # noqa: B006
        x_token: Annotated[str, Header()] = "",
        session: Annotated[str, Cookie()] = "",
    ):
        return [item_id, limit, missing, unset, tag, x_token, session]

    async def asynchronous(
        item_id: int,
        limit: int,
        missing: str = "fallback",
        unset: str | None = None,
        tag: Annotated[list[str], Query()] = ["none"],  # noqa: B006
        x_token: Annotated[str, Header()] = "",
        session: Annotated[str, Cookie()] = "",
    ):
        return [item_id, limit, missing, unset, tag, x_token, session]

    client = client_for(asynchronous if wrap else plain, "/items/{item_id}")

    response = client.get(
        "/items/7",
        params={"limit": "2", "tag": ["a", "b"]},
        headers={"X-Token": "secret"},
        cookies={"session": "abc"},
    )

    assert response.json() == [7, 2, "fallback", None, ["a", "b"], "secret", "abc"]


def test_every_method_reaches_its_handler():
    app = App()
    for method in ("get", "post", "put", "delete", "patch", "options"):
        getattr(app, method)("/items")(lambda method=method: None)
    client = TestClient(app)

    for method in ("get", "post", "put", "delete", "patch", "options"):
        assert getattr(client, method)("/items").status_code == 200


def test_a_head_request_answers_no_body():
    def handler():
        return {"not": "sent"}

    response = client_for(handler, method="head").head("/items")

    assert response.status_code == 200
    assert response.content == b""


def test_a_body_arrives_as_the_model_with_its_defaults_and_nested_models():
    def handler(item: Item):
        return {
            "is_item": isinstance(item, Item),
            "name": item.name,
            "price": item.price,
            "tag_types": [type(tag).__name__ for tag in item.tags],
            "parent": item.parent and item.parent.name,
        }

    response = client_for(handler, method="post").post(
        "/items",
        json={"name": "a", "tags": [{"name": "x"}], "parent": {"name": "y"}},
    )

    assert response.json() == {
        "is_item": True,
        "name": "a",
        "price": 1.5,
        "tag_types": ["Tag"],
        "parent": "y",
    }


def test_a_null_in_an_optional_model_field_is_none():
    def handler(item: Item):
        return item.parent is None

    client = client_for(handler, method="post")

    assert client.post("/items", json={"name": "a", "parent": None}).json() is True


def test_a_body_may_be_a_list_of_models_or_a_marked_scalar():
    def models(items: list[Tag]):
        return [type(item).__name__ for item in items]

    def scalar(count: Annotated[int, Body()]):
        return count + 1

    app = App()
    app.post("/models")(models)
    app.post("/scalar")(scalar)
    client = TestClient(app)

    assert client.post("/models", json=[{"name": "a"}]).json() == ["Tag"]
    assert client.post("/scalar", json=1).json() == 2


def test_an_annotated_body_is_read_as_its_model():
    async def handler(item: Annotated[Tag, Body()]):
        return type(item).__name__

    response = client_for(handler, method="post").post("/items", json={"name": "a"})

    assert response.json() == "Tag"


def test_a_body_field_the_model_does_not_declare_is_left_out():
    def handler(tag: Tag):
        return tag.name

    client = client_for(handler, method="post")

    assert client.post("/items", json={"name": "a", "other": 1}).json() == "a"


def test_a_field_that_is_not_an_init_argument_stays_out_of_the_body():
    @dataclass
    class Computed:
        name: str
        size: int = field(init=False, default=7)

    def handler(computed: Computed):
        return computed.size

    client = client_for(handler, method="post")

    assert client.post("/items", json={"name": "a", "size": 1}).json() == 7


def test_a_null_for_an_optional_list_of_models_is_none():
    @dataclass
    class Group:
        tags: list[Tag] | None = None

    def handler(group: Group):
        return group.tags and [type(tag).__name__ for tag in group.tags]

    client = client_for(handler, method="post")

    assert client.post("/items", json={"tags": None}).json() is None
    assert client.post("/items", json={"tags": [{"name": "a"}]}).json() == ["Tag"]


def test_a_list_of_plain_values_in_a_model_arrives_as_sent():
    @dataclass
    class Labels:
        names: list[str]

    def handler(labels: Labels):
        return labels.names

    client = client_for(handler, method="post")

    assert client.post("/items", json={"names": ["a", "b"]}).json() == ["a", "b"]


@dataclass
class Node:
    name: str
    child: "Node | None" = None


def test_a_model_inside_itself_arrives_as_that_model_at_every_level():
    def handler(node: Node):
        return [type(node.child).__name__, node.child.child.child]

    client = client_for(handler, method="post")
    body = {"name": "a", "child": {"name": "b", "child": {"name": "c"}}}

    assert client.post("/items", json=body).json() == ["Node", None]


def test_a_returned_model_answers_its_fields_including_nested_ones():
    def handler():
        return [Item("a", tags=[Tag("x")])]

    assert client_for(handler).get("/items").json() == [
        {"name": "a", "price": 1.5, "tags": [{"name": "x"}], "parent": None}
    ]


def test_a_value_that_failed_validation_never_reaches_the_handler():
    calls = []

    def handler(limit: int):
        calls.append(limit)

    response = client_for(handler).get("/items", params={"limit": "many"})

    assert response.status_code == 422
    assert response.json()["detail"][0]["loc"] == ["query", "limit"]
    assert calls == []


def test_a_large_integer_and_a_float_cross_as_python_numbers():
    def handler(item: list[Any]):
        return [type(value).__name__ for value in item]

    response = client_for(handler, method="post").post(
        "/items", json=[18446744073709551615, 1.5, True, None, "x", {"a": 1}, [1]]
    )

    assert response.json() == [
        "int",
        "float",
        "bool",
        "NoneType",
        "str",
        "dict",
        "list",
    ]


def test_a_failed_handler_answers_500_and_logs_when_told_not_to_raise(caplog):
    def handler():
        raise ValueError("boom")

    client = client_for(handler, raise_server_exceptions=False)

    with caplog.at_level(logging.ERROR, logger="thermite.error"):
        response = client.get("/items")

    assert response.status_code == 500
    assert response.text == "Internal Server Error"
    assert response.headers["content-type"] == "text/plain; charset=utf-8"
    assert "Exception in handler `handler`" in caplog.text
    assert "ValueError: boom" in caplog.text


def test_a_failed_async_handler_answers_500():
    async def raises():
        raise ValueError("boom")

    client = client_for(raises, raise_server_exceptions=False)

    assert client.get("/items").status_code == 500


@pytest.mark.skipif(
    sys.version_info < (3, 12), reason="markcoroutinefunction is new in 3.12"
)
def test_a_handler_marked_async_that_gives_no_coroutine_answers_500():
    def not_a_coroutine():
        return "plain"

    client = client_for(
        inspect.markcoroutinefunction(not_a_coroutine), raise_server_exceptions=False
    )

    assert client.get("/items").status_code == 500


def test_an_answer_that_is_not_json_answers_500():
    def plain():
        return object()

    async def asynchronous():
        return object()

    app = App()
    app.get("/plain")(plain)
    app.get("/async")(asynchronous)
    client = TestClient(app, raise_server_exceptions=False)

    assert client.get("/plain").status_code == 500
    assert client.get("/async").status_code == 500


def test_a_body_a_model_rejects_answers_500_on_either_kind_of_handler():
    @dataclass
    class Strict:
        value: int

        def __post_init__(self):
            raise ValueError("rejected")

    def plain(item: Strict):
        pass

    async def asynchronous(item: Strict):
        pass

    app = App()
    app.post("/plain")(plain)
    app.post("/async")(asynchronous)
    client = TestClient(app, raise_server_exceptions=False)

    assert client.post("/plain", json={"value": 1}).status_code == 500
    assert client.post("/async", json={"value": 1}).status_code == 500


def test_a_value_json_cannot_hold_answers_500():
    def handler():
        return float("nan")

    client = client_for(handler, raise_server_exceptions=False)

    assert client.get("/items").status_code == 500


@pytest.mark.parametrize("kind", ["plain", "async"])
def test_the_client_raises_what_the_handler_raised(kind):
    def plain():
        raise ValueError("boom")

    async def asynchronous():
        raise ValueError("boom")

    client = client_for(plain if kind == "plain" else asynchronous)

    with pytest.raises(ValueError, match="boom"):
        client.get("/items")


def test_system_exit_in_a_handler_does_not_take_the_server_down():
    def handler():
        raise SystemExit(1)

    client = client_for(handler, raise_server_exceptions=False)

    assert client.get("/items").status_code == 500


def test_async_handlers_run_at_the_same_time():
    both_started = asyncio.Event()
    started = []

    async def handler(name: str):
        started.append(name)
        if len(started) == 2:
            both_started.set()
        await asyncio.wait_for(both_started.wait(), timeout=5)
        return name

    client = client_for(handler)

    with ThreadPoolExecutor(2) as executor:
        answers = list(
            executor.map(
                lambda name: client.get("/items", params={"name": name}).json(), "ab"
            )
        )

    assert answers == ["a", "b"]


def test_a_blocking_handler_leaves_async_handlers_running():
    finish = threading.Event()

    def blocking():
        return finish.wait(timeout=5)

    async def quick():
        finish.set()
        return "quick"

    app = App()
    app.get("/blocking")(blocking)
    app.get("/quick")(quick)
    client = TestClient(app)

    with ThreadPoolExecutor(1) as executor:
        blocked = executor.submit(client.get, "/blocking")
        assert client.get("/quick").json() == "quick"

    assert blocked.result().json() is True


def test_plain_handlers_run_on_another_thread_and_async_ones_on_the_loop():
    def plain():
        return threading.get_ident()

    async def asynchronous():
        asyncio.get_running_loop()
        return threading.get_ident()

    app = App()
    app.get("/plain")(plain)
    app.get("/async")(asynchronous)
    client = TestClient(app)

    first = client.get("/async").json()

    assert first == client.get("/async").json()
    assert client.get("/plain").json() != threading.get_ident()
    assert first != threading.get_ident()


def test_a_404_and_a_405_still_come_from_the_core():
    def handler():
        pass

    client = client_for(handler)

    assert client.get("/other").status_code == 404
    assert client.post("/items").status_code == 405


def test_the_client_serves_the_openapi_document_and_docs():
    def handler():
        pass

    client = client_for(handler)

    assert "/items" in client.get("/openapi.json").json()["paths"]
    assert "swagger" in client.get("/docs").text.lower()


def test_a_path_with_a_space_and_non_ascii_text_is_quoted():
    def handler(name: str):
        return name

    client = client_for(handler, "/items/{name}")

    assert client.get("/items/a b ä").json() == "a b ä"


def test_a_query_in_the_url_is_kept():
    def handler(limit: int):
        return limit

    assert client_for(handler).get("/items?limit=3").json() == 3


def test_repeated_response_headers_would_be_joined():
    from thermite.testclient import _join_repeated

    assert _join_repeated([("a", "1"), ("b", "2"), ("a", "3")]) == {
        "a": "1, 3",
        "b": "2",
    }


def test_the_client_works_as_a_context_manager():
    def handler():
        return 1

    with client_for(handler) as client:
        assert client.get("/items").json() == 1


def test_an_invalid_header_is_a_value_error():
    def handler():
        pass

    with pytest.raises(ValueError):
        client_for(handler).get("/items", headers={"bad name": "x"})


def test_a_route_cannot_be_added_while_a_client_holds_the_app():
    def handler():
        pass

    app = App()
    client = TestClient(app)

    with pytest.raises(RouteError, match="cannot add `/items`"):
        app.get("/items")(handler)

    del client
    app.get("/items")(handler)
    assert TestClient(app).get("/items").status_code == 200


def test_a_route_shadowing_the_document_stops_the_client_from_starting():
    def handler():
        pass

    with pytest.raises(RouteError, match="/openapi.json"):
        client_for(handler, "/openapi.json")


def test_a_handler_waiting_on_what_only_it_holds_survives_a_garbage_collection():
    async def handler():
        loop = asyncio.get_running_loop()
        waited = loop.create_future()
        reference = weakref.ref(waited)

        def wake():
            future = reference()
            if future is not None:
                future.set_result("woken")

        loop.call_soon(gc.collect)
        loop.call_later(0.05, wake)
        return await waited

    response = client_for(handler).get("/items")

    assert response.status_code == 200
    assert response.json() == "woken"


def test_async_requests_sent_together_are_all_answered_on_the_event_loop():
    threads = set()

    async def handler(number: int):
        threads.add(threading.get_ident())
        await asyncio.sleep(0)
        return number

    client = client_for(handler, path="/items/{number}")
    with ThreadPoolExecutor(32) as pool:
        answers = list(
            pool.map(lambda n: client.get(f"/items/{n}").json(), range(2000))
        )

    assert answers == list(range(2000))
    assert len(threads) == 1


def test_a_def_handler_does_not_hold_up_an_async_one():
    released = threading.Event()

    def blocking():
        assert released.wait(5)
        return "released"

    async def quick():
        released.set()
        return "quick"

    app = App()
    app.get("/blocking")(blocking)
    app.get("/quick")(quick)
    client = TestClient(app)
    with ThreadPoolExecutor(1) as pool:
        blocked = pool.submit(client.get, "/blocking")
        assert client.get("/quick").json() == "quick"
        assert blocked.result().json() == "released"
