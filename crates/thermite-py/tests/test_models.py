from dataclasses import dataclass, field
from typing import Annotated

import pytest
from thermite import App, Body, Query, RouteError


@dataclass
class Item:
    name: str
    price: float = 1.5
    tags: list[str] = field(default_factory=list)
    note: str | None = None
    hidden: int = field(default=0, init=False)


@dataclass
class Tree:
    value: int
    children: "list[Tree]" = field(default_factory=list)


def operation_of(handler, method="post"):
    app = App()
    getattr(app, method)("/items")(handler)
    document = app.openapi()
    return document["paths"]["/items"][method], document.get("components", {}).get(
        "schemas", {}
    )


def body_schema(operation):
    return operation["requestBody"]["content"]["application/json"]["schema"]


def response_schema(operation):
    return operation["responses"]["200"]["content"]["application/json"]["schema"]


def test_a_dataclass_parameter_is_a_required_body():
    def handler(item: Item):
        pass

    operation, _ = operation_of(handler)

    assert body_schema(operation) == {"$ref": "#/components/schemas/Item"}
    assert operation["requestBody"]["required"] is True
    assert "parameters" not in operation


def test_a_dataclass_gives_the_fields_of_its_model():
    def handler(item: Item):
        pass

    _, schemas = operation_of(handler)
    model = schemas["Item"]

    assert list(model["properties"]) == ["name", "price", "tags", "note"]
    assert model["required"] == ["name"]
    assert model["properties"]["name"]["type"] == "string"
    assert model["properties"]["price"]["default"] == 1.5
    assert model["properties"]["tags"]["default"] == []
    assert {"type": "null"} in model["properties"]["note"]["anyOf"]


def test_a_model_inside_itself_is_a_reference():
    def handler(tree: Tree):
        pass

    _, schemas = operation_of(handler)

    assert schemas["Tree"]["properties"]["children"]["items"] == {
        "$ref": "#/components/schemas/Tree"
    }


@pytest.mark.parametrize("annotation", [list[Item], list[int], list])
def test_an_unmarked_list_is_a_body(annotation):
    def handler(items: annotation):
        pass

    operation, _ = operation_of(handler)

    assert body_schema(operation)["type"] == "array"


def test_a_marked_scalar_is_a_body():
    def handler(count: Annotated[int, Body()]):
        pass

    operation, _ = operation_of(handler)

    assert body_schema(operation)["type"] == "integer"


def test_a_marker_called_or_not_reads_the_body():
    def handler(count: Annotated[int, Body]):
        pass

    operation, _ = operation_of(handler)

    assert body_schema(operation)["type"] == "integer"


def test_a_field_may_be_annotated():
    @dataclass
    class Marked:
        count: Annotated[int, "a note"]

    def handler(marked: Marked):
        pass

    _, schemas = operation_of(handler)

    assert schemas["Marked"]["properties"]["count"]["type"] == "integer"


def test_an_optional_body_is_rejected():
    def handler(item: Item | None = None):
        pass

    with pytest.raises(RouteError, match="optional body"):
        App().post("/items")(handler)


def test_a_second_body_is_rejected():
    def handler(first: Item, second: Tree):
        pass

    with pytest.raises(RouteError, match="handler `handler` declares more than one"):
        App().post("/items")(handler)


def test_the_core_rejects_a_model_in_the_query():
    def handler(item: Annotated[Item, Query()]):
        pass

    with pytest.raises(RouteError, match="query parameter `item`"):
        App().post("/items")(handler)


def test_the_core_rejects_two_models_of_one_name():
    def make():
        @dataclass
        class Twin:
            first: int

        return Twin

    @dataclass
    class Twin:
        second: str

    @dataclass
    class Pair:
        one: make()
        other: Twin

    def handler(pair: Pair):
        pass

    with pytest.raises(RouteError, match="two different models named `Twin`"):
        App().post("/items")(handler)


def test_a_field_of_an_unsupported_type_is_rejected():
    @dataclass
    class Wrong:
        values: dict[str, int]

    def handler(wrong: Wrong):
        pass

    with pytest.raises(RouteError, match=r"parameter `wrong`.*not supported yet"):
        App().post("/items")(handler)


def test_a_model_with_an_unknown_forward_reference_is_rejected():
    @dataclass
    class Wrong:
        value: "Missing"  # noqa: F821

    def handler(wrong: Wrong):
        pass

    with pytest.raises(RouteError, match=r"model `Wrong`.*Missing"):
        App().post("/items")(handler)


def test_a_plain_class_is_rejected():
    class Plain:
        pass

    def handler(plain: Plain):
        pass

    with pytest.raises(RouteError, match="not supported yet"):
        App().post("/items")(handler)


def test_the_return_annotation_documents_the_response():
    def handler() -> Item:
        pass

    operation, schemas = operation_of(handler, "get")

    assert response_schema(operation) == {"$ref": "#/components/schemas/Item"}
    assert "Item" in schemas


def test_a_return_annotation_may_be_a_list_or_a_scalar():
    def items() -> list[Item]:
        pass

    def count() -> int:
        pass

    assert response_schema(operation_of(items, "get")[0])["type"] == "array"
    assert response_schema(operation_of(count, "get")[0])["type"] == "integer"


@pytest.mark.parametrize("annotated", [False, True])
def test_no_return_annotation_or_none_documents_any_response(annotated):
    def handler():
        pass

    if annotated:
        handler.__annotations__["return"] = None

    assert response_schema(operation_of(handler, "get")[0]) == {}


def test_an_unsupported_return_annotation_is_rejected():
    def handler() -> dict[str, int]:
        pass

    with pytest.raises(RouteError, match="return type of `handler`"):
        App().get("/items")(handler)
