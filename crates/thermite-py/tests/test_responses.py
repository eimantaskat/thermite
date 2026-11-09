import logging
from dataclasses import dataclass, field
from typing import Any

import pytest

from thermite import App, ResponseValidationError
from thermite.testclient import TestClient


@dataclass
class Item:
    id: int
    price: float
    note: str | None = None
    size: int = 1


@dataclass
class Secret(Item):
    password: str = "hunter2"


def client_for(handler, **options):
    app = App()
    app.get("/items")(handler)
    return TestClient(app, **options)


def sync_and_async(answer):
    def sync_handler() -> Item:
        return answer()

    async def async_handler() -> Item:
        return answer()

    return [sync_handler, async_handler]


@pytest.mark.parametrize("kind", ["sync", "async"])
def test_a_returned_dict_is_validated_and_filtered_by_the_return_annotation(kind):
    sent = {"price": 3, "extra": True, "id": "5"}
    handler = sync_and_async(lambda: sent)[kind == "async"]

    response = client_for(handler).get("/items")

    assert response.status_code == 200
    assert response.text == '{"id":5,"price":3.0,"note":null,"size":1}'


def test_a_returned_subclass_answers_only_the_fields_of_the_annotation():
    def handler() -> Item:
        return Secret(id=1, price=2.5)

    response = client_for(handler).get("/items")

    assert response.json() == {"id": 1, "price": 2.5, "note": None, "size": 1}


def test_each_item_of_a_returned_list_is_validated():
    def handler() -> list[Item]:
        return [Secret(id=1, price=2.5), {"id": 2, "price": "4.5", "size": "3"}]

    response = client_for(handler).get("/items")

    assert response.json() == [
        {"id": 1, "price": 2.5, "note": None, "size": 1},
        {"id": 2, "price": 4.5, "note": None, "size": 3},
    ]


@pytest.mark.parametrize("kind", ["sync", "async"])
def test_a_return_value_that_does_not_match_answers_500_and_logs(kind, caplog):
    handler = sync_and_async(lambda: {"id": "five", "note": 3})[kind == "async"]

    with caplog.at_level(logging.ERROR, logger="thermite.error"):
        response = client_for(handler, raise_server_exceptions=False).get("/items")

    assert response.status_code == 500
    [record] = caplog.records
    assert isinstance(record.exc_info[1], ResponseValidationError)
    assert "3 validation errors in the response" in record.exc_text


def test_the_client_raises_the_errors_of_a_return_value_that_does_not_match():
    def handler() -> Item:
        return {"id": "five", "price": 1}

    with pytest.raises(ResponseValidationError) as raised:
        client_for(handler).get("/items")

    assert raised.value.errors() == [
        {
            "type": "int_parsing",
            "loc": ["response", "id"],
            "msg": (
                "Input should be a valid integer, unable to parse string as an integer"
            ),
            "input": "five",
        }
    ]
    assert str(raised.value) == (
        "1 validation error in the response\n"
        "  {'type': 'int_parsing', 'loc': ['response', 'id'], 'msg': 'Input should be "
        "a valid integer, unable to parse string as an integer', 'input': 'five'}"
    )


def test_a_scalar_return_annotation_coerces_and_rejects_like_a_field():
    def handler() -> int:
        return "7"

    def wrong() -> int:
        return [7]

    assert client_for(handler).get("/items").json() == 7
    assert (
        client_for(wrong, raise_server_exceptions=False).get("/items").status_code
        == 500
    )


@pytest.mark.parametrize("annotation", [Any, "missing"])
def test_without_a_return_annotation_or_with_any_the_value_answers_as_it_is(
    annotation,
):
    def handler():
        return {"id": "five", "extra": [1, None]}

    if annotation is Any:
        handler.__annotations__["return"] = Any

    assert client_for(handler).get("/items").json() == {
        "id": "five",
        "extra": [1, None],
    }


def test_a_missing_field_with_a_default_factory_is_left_out_of_a_returned_dict():
    @dataclass
    class Tagged:
        name: str
        tags: list[str] = field(default_factory=list)

    def handler() -> Tagged:
        return {"name": "pen"}

    assert client_for(handler).get("/items").json() == {"name": "pen"}


def test_a_returned_non_ascii_string_survives_validation():
    def handler() -> str:
        return "žąsis ✓"

    assert client_for(handler).get("/items").json() == "žąsis ✓"
