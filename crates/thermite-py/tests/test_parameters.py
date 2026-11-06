from typing import Annotated, Any, Optional

import pytest
from thermite import App, Body, Cookie, Header, Path, Query, RouteError


def parameters_of(handler, path="/items"):
    app = App()
    app.get(path)(handler)
    operation = app.openapi()["paths"][path]["get"]
    return {
        parameter["name"]: parameter for parameter in operation.get("parameters", [])
    }


def test_a_name_in_the_path_is_a_path_parameter():
    def handler(item_id: int):
        pass

    parameter = parameters_of(handler, "/items/{item_id}")["item_id"]

    assert parameter["in"] == "path"
    assert parameter["required"] is True
    assert parameter["schema"]["type"] == "integer"


def test_any_other_scalar_is_a_required_query_parameter():
    def handler(limit: int):
        pass

    parameter = parameters_of(handler)["limit"]

    assert parameter["in"] == "query"
    assert parameter["required"] is True


@pytest.mark.parametrize(
    ("annotation", "expected"),
    [(str, "string"), (int, "integer"), (float, "number"), (bool, "boolean")],
)
def test_scalars_map_to_their_json_types(annotation, expected):
    def handler(value: annotation):
        pass

    assert parameters_of(handler)["value"]["schema"]["type"] == expected


def test_a_parameter_with_no_annotation_or_any_takes_any_value():
    def handler(first, second: Any):
        pass

    parameters = parameters_of(handler)

    assert "type" not in parameters["first"]["schema"]
    assert "type" not in parameters["second"]["schema"]


def test_a_default_makes_a_parameter_optional_and_is_documented():
    def handler(limit: int = 10):
        pass

    parameter = parameters_of(handler)["limit"]

    assert parameter["required"] is False
    assert parameter["schema"]["default"] == 10


@pytest.mark.parametrize("annotation", [str | None, Optional[str]])
def test_none_as_default_makes_a_nullable_optional_parameter(annotation):
    def handler(q: annotation = None):
        pass

    parameter = parameters_of(handler)["q"]

    assert parameter["required"] is False
    assert {"type": "null"} in parameter["schema"]["anyOf"]


def test_none_as_default_is_accepted_without_none_in_the_type():
    def handler(q: str = None):
        pass

    assert parameters_of(handler)["q"]["required"] is False


def test_an_integer_default_is_accepted_for_a_float():
    def handler(ratio: float = 1):
        pass

    assert parameters_of(handler)["ratio"]["schema"]["default"] == 1.0


def test_a_default_on_a_nullable_type_keeps_the_default():
    def handler(limit: int | None = 5):
        pass

    assert parameters_of(handler)["limit"]["schema"]["default"] == 5


def test_markers_choose_the_location():
    def handler(
        item_id: Annotated[int, Path()],
        limit: Annotated[int, Query()],
        token: Annotated[str, Header()],
        session: Annotated[str, Cookie()],
    ):
        pass

    parameters = parameters_of(handler, "/items/{item_id}")

    assert parameters["item_id"]["in"] == "path"
    assert parameters["limit"]["in"] == "query"
    assert parameters["token"]["in"] == "header"
    assert parameters["session"]["in"] == "cookie"


def test_a_marker_works_without_being_called():
    def handler(token: Annotated[str, Header]):
        pass

    assert parameters_of(handler)["token"]["in"] == "header"


def test_other_annotated_metadata_is_ignored():
    def handler(limit: Annotated[int, "a note"]):
        pass

    parameter = parameters_of(handler)["limit"]

    assert parameter["in"] == "query"
    assert parameter["schema"]["type"] == "integer"


def test_a_header_parameter_turns_underscores_into_hyphens():
    def handler(x_token: Annotated[str, Header()]):
        pass

    assert "x-token" in parameters_of(handler)


def test_a_query_parameter_keeps_its_underscores():
    def handler(page_size: int):
        pass

    assert "page_size" in parameters_of(handler)


def test_a_marked_list_is_a_query_parameter():
    def handler(tag: Annotated[list[str], Query()]):
        pass

    parameter = parameters_of(handler)["tag"]

    assert parameter["in"] == "query"
    assert parameter["schema"]["items"]["type"] == "string"


def test_string_annotations_are_resolved():
    def handler(limit: "int"):
        pass

    assert parameters_of(handler)["limit"]["schema"]["type"] == "integer"


@pytest.mark.parametrize(
    "handler",
    [
        pytest.param(lambda first, /: None, id="positional only"),
        pytest.param(lambda *values: None, id="variable positional"),
        pytest.param(lambda **values: None, id="variable keyword"),
    ],
)
def test_a_parameter_that_cannot_be_passed_by_name_is_rejected(handler):
    with pytest.raises(RouteError, match="must be passable by name"):
        App().get("/items")(handler)


def test_a_marker_as_the_default_is_rejected():
    def handler(limit: int = Query()):
        pass

    with pytest.raises(RouteError, match=r"parameter `limit` of `handler`.*Annotated"):
        App().get("/items")(handler)


def test_two_markers_on_one_parameter_are_rejected():
    def handler(limit: Annotated[int, Query(), Header()]):
        pass

    with pytest.raises(RouteError, match="only one of"):
        App().get("/items")(handler)


def test_a_required_nullable_parameter_is_rejected_with_the_way_out():
    def handler(limit: int | None):
        pass

    with pytest.raises(RouteError, match=r"parameter `limit`.*write `= None`"):
        App().get("/items")(handler)


def test_a_union_is_rejected():
    def handler(value: int | str = 1):
        pass

    with pytest.raises(RouteError, match="union .* is not supported yet"):
        App().get("/items")(handler)


@pytest.mark.parametrize("annotation", [dict[str, int], bytes, tuple[int, str]])
def test_an_unsupported_type_is_rejected_by_name(annotation):
    def handler(value: Annotated[annotation, Query()]):
        pass

    with pytest.raises(RouteError, match=r"parameter `value`.*not supported yet"):
        App().get("/items")(handler)


def test_an_unhashable_annotation_is_rejected_like_any_other():
    def handler(value: ["not", "a", "type"]):
        pass

    with pytest.raises(RouteError, match="not supported yet"):
        App().get("/items")(handler)


def test_a_default_that_is_not_json_is_rejected():
    def handler(value: Annotated[str, Query()] = object()):
        pass

    with pytest.raises(RouteError, match="is not a JSON value"):
        App().get("/items")(handler)


def test_a_default_that_is_not_finite_is_rejected():
    def handler(value: float = float("nan")):
        pass

    with pytest.raises(RouteError, match="is not a JSON value"):
        App().get("/items")(handler)


def test_an_unknown_forward_reference_is_rejected():
    def handler(value: "Missing"):  # noqa: F821
        pass

    with pytest.raises(RouteError, match=r"handler `handler`.*Missing"):
        App().get("/items")(handler)


def test_the_core_rejects_a_default_of_the_wrong_type():
    def handler(limit: int = "ten"):
        pass

    with pytest.raises(RouteError, match="default for `limit`"):
        App().get("/items")(handler)


def test_the_core_rejects_a_path_segment_with_no_parameter():
    def handler():
        pass

    with pytest.raises(RouteError, match="no path parameter `item_id`"):
        App().get("/items/{item_id}")(handler)


def test_the_core_rejects_a_marked_path_parameter_missing_from_the_path():
    def handler(item_id: Annotated[int, Path()]):
        pass

    with pytest.raises(RouteError, match="no segment"):
        App().get("/items")(handler)


def test_the_core_rejects_an_optional_path_parameter():
    def handler(item_id: int = 1):
        pass

    with pytest.raises(RouteError, match="cannot be optional"):
        App().get("/items/{item_id}")(handler)


def test_the_core_rejects_a_route_that_matches_an_existing_one():
    def handler():
        pass

    app = App()
    app.get("/items")(handler)

    with pytest.raises(RouteError, match="matches the same requests"):
        app.get("/items")(handler)


def test_a_failed_route_leaves_the_app_as_it_was():
    def good():
        pass

    def bad(limit: int = "ten"):
        pass

    app = App()
    app.get("/good")(good)

    with pytest.raises(RouteError):
        app.get("/bad")(bad)

    assert list(app.openapi()["paths"]) == ["/good"]


def test_a_route_shadowing_the_document_is_reported_when_it_is_built():
    def handler():
        pass

    app = App()
    app.get("/openapi.json")(handler)

    with pytest.raises(RouteError, match="/openapi.json"):
        app.openapi()


def test_body_is_not_a_location_for_a_parameter():
    def handler(count: Annotated[int, Body()]):
        pass

    assert parameters_of(handler) == {}
