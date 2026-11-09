import asyncio
import logging
import threading
from dataclasses import KW_ONLY, dataclass, field, make_dataclass
from typing import Any

import pytest

from thermite import App
from thermite.testclient import TestClient


def client_for(handler, **options):
    app = App()
    app.post("/items")(handler)
    return TestClient(app, **options)


def on_event_loop():
    try:
        asyncio.get_running_loop()
    except RuntimeError:
        return False
    return True


def test_a_keyword_only_field_arrives_by_name():
    @dataclass
    class Item:
        name: str
        _: KW_ONLY
        price: float
        count: int = 1

    def handler(item: Item):
        return [item.name, item.price, item.count]

    response = client_for(handler).post("/items", json={"price": 2, "name": "pen"})

    assert response.json() == ["pen", 2.0, 1]


def test_a_body_of_plain_values_arrives_as_dicts_and_lists():
    @dataclass
    class Envelope:
        payload: Any
        extra: list

    def handler(envelope: Envelope):
        return [envelope.payload, envelope.extra]

    sent = {"payload": {"a": [1, {"b": None}], "c": "d"}, "extra": [{"e": 1.5}, []]}

    response = client_for(handler).post("/items", json=sent)

    assert response.json() == [sent["payload"], sent["extra"]]


@dataclass
class Point:
    x: int
    y: int
    labels: list[str] = field(default_factory=list)


@dataclass
class Shape:
    name: str
    points: list[Point]


# enough points that building them takes several turns of the event loop
POINTS = [{"x": index, "y": -index, "labels": ["a", "b"]} for index in range(20_000)]


@pytest.mark.parametrize(
    ("annotation", "body", "points_of"),
    [
        (list[Point], POINTS, lambda value: value),
        (Shape, {"name": "line", "points": POINTS}, lambda value: value.points),
    ],
)
def test_a_large_body_is_built_in_turns_that_let_the_event_loop_run(
    annotation, body, points_of
):
    turns = []
    stop = threading.Event()

    def count_turns():
        turns.append(None)
        if not stop.is_set():
            asyncio.get_running_loop().call_soon(count_turns)

    @dataclass
    class First:
        def __post_init__(self):
            count_turns()

    def count_when_built(self):
        stop.set()
        self.turns = len(turns)

    # fields are built in order and the model last, so counting spans the large value
    wrapped = make_dataclass(
        "Wrapped",
        [("first", First), ("value", annotation)],
        namespace={"__post_init__": count_when_built},
    )

    async def handler(wrapped: wrapped):
        points = points_of(wrapped.value)
        return [wrapped.turns, len(points), points[-1].x, points[-1].labels]

    response = client_for(handler).post("/items", json={"first": {}, "value": body})
    turns_seen, *answer = response.json()

    assert answer == [20_000, 19_999, ["a", "b"]]
    assert turns_seen > 1


def test_a_large_body_for_a_def_handler_is_built_on_the_event_loop():
    built_where = []

    @dataclass
    class Marked:
        index: int

        def __post_init__(self):
            if self.index == 0:
                built_where.append(on_event_loop())

    def handler(items: list[Marked]):
        return [len(items), on_event_loop()]

    client = client_for(handler)
    small = client.post("/items", json=[{"index": 0}]).json()
    large = client.post("/items", json=[{"index": i} for i in range(20_000)]).json()

    assert (small, large) == ([1, False], [20_000, False])
    assert built_where == [False, True]


def test_a_large_body_a_model_rejects_answers_500_for_a_def_handler(caplog):
    @dataclass
    class Strict:
        value: int

        def __post_init__(self):
            if self.value == 9_999:
                raise ValueError("rejected")

    def handler(items: list[Strict]):
        pass

    client = client_for(handler, raise_server_exceptions=False)
    with caplog.at_level(logging.ERROR, logger="thermite.error"):
        response = client.post("/items", json=[{"value": i} for i in range(20_000)])

    assert response.status_code == 500
    assert "Exception in handler `handler`" in caplog.text
    assert "rejected" in caplog.text


def test_a_default_factory_runs_for_each_request_and_not_at_registration():
    made = []

    def stamp():
        made.append(None)
        return len(made)

    @dataclass
    class Order:
        name: str
        number: int = field(default_factory=stamp)

    def handler(order: Order):
        return order.number

    client = client_for(handler)
    registered = len(made)
    first = client.post("/items", json={"name": "pen"}).json()
    second = client.post("/items", json={"name": "ink"}).json()
    sent = client.post("/items", json={"name": "nib", "number": "40"}).json()

    assert registered == 0
    assert [first, second, sent] == [1, 2, 40]


def test_fields_after_a_missing_one_with_a_factory_arrive_by_name():
    @dataclass
    class Point:
        x: int
        labels: list[str] = field(default_factory=list)
        y: int = 0
        tags: list[str] = field(default_factory=lambda: ["new"])
        z: int = 0

    def handler(points: list[Point]):
        return [
            [point.x, point.labels, point.y, point.tags, point.z] for point in points
        ]

    response = client_for(handler).post(
        "/items",
        json=[{"x": 1, "y": 2}, {"z": 3, "labels": ["a"], "x": 4}, {"x": 5}],
    )

    assert response.json() == [
        [1, [], 2, ["new"], 0],
        [4, ["a"], 0, ["new"], 3],
        [5, [], 0, ["new"], 0],
    ]


def test_a_field_with_a_factory_still_rejects_a_value_of_the_wrong_type():
    def handler(point: Point):
        return point.labels

    response = client_for(handler).post("/items", json={"x": 1, "y": 2, "labels": None})

    assert response.status_code == 422
    assert response.json()["detail"][0]["loc"] == ["body", "labels"]
