import pytest
from thermite._thermite import CoreApp, Field, Schema


def test_a_default_that_is_not_json_text_is_a_value_error():
    with pytest.raises(ValueError, match="expected"):
        Field.with_default("limit", Schema.integer(), "not json")


def test_an_invalid_method_is_a_value_error():
    with pytest.raises(ValueError, match="invalid HTTP method"):
        CoreApp().add_route("NOT A METHOD", "/items", "items", [], None, None)
