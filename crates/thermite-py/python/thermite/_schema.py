import dataclasses
import inspect
import json
import types
import typing
from collections.abc import Callable
from typing import Annotated, Any, TypeGuard, Union

from thermite._thermite import Field, RouteError, Schema

NO_DEFAULT = inspect.Parameter.empty
# a dataclass field whose default is made per request by its `default_factory`
DEFAULT_FACTORY = object()

_SCALARS: dict[object, Callable[[], Schema]] = {
    str: Schema.string,
    int: Schema.integer,
    float: Schema.number,
    bool: Schema.boolean,
    Any: Schema.any,
    inspect.Parameter.empty: Schema.any,
}


def schema_of(annotation: object, enclosing: tuple[type, ...] = ()) -> Schema:
    annotation = strip_annotated(annotation)
    if annotation is list:
        return Schema.array(Schema.any())
    if typing.get_origin(annotation) is list:
        return Schema.array(schema_of(typing.get_args(annotation)[0], enclosing))
    if is_model(annotation):
        return _model_schema(annotation, enclosing)
    if _is_hashable(annotation) and annotation in _SCALARS:
        return _SCALARS[annotation]()
    raise RouteError(f"the type {annotation!r} is not supported yet")


def field_of(
    name: str,
    annotation: object,
    default: object,
    enclosing: tuple[type, ...] = (),
) -> Field:
    inner, nullable = split_none(annotation)
    schema = schema_of(inner, enclosing)
    if default is NO_DEFAULT:
        if nullable:
            raise RouteError(
                f"{annotation!r} needs a default: a required value that may be null "
                "is not supported yet, so write `= None`"
            )
        return Field.required(name, schema)
    if default is DEFAULT_FACTORY:
        return Field.with_default_factory(name, schema)
    if default is None:
        return Field.optional(name, schema)
    # the core rejects a default that validating would change, and 1 becomes 1.0
    if inner is float and type(default) is int:
        default = float(default)
    return Field.with_default(name, schema, _default_json(default))


# `int | None` as (int, True); schemas have no unions, so only None can go
def split_none(annotation: object) -> tuple[object, bool]:
    annotation = strip_annotated(annotation)
    if typing.get_origin(annotation) not in (Union, types.UnionType):
        return annotation, False
    others = [arg for arg in typing.get_args(annotation) if arg is not type(None)]
    if len(others) != 1:
        raise RouteError(f"the union {annotation!r} is not supported yet")
    return others[0], True


def strip_annotated(annotation: object) -> object:
    if typing.get_origin(annotation) is Annotated:
        return typing.get_args(annotation)[0]
    return annotation


def is_model(annotation: object) -> TypeGuard[type]:
    return isinstance(annotation, type) and dataclasses.is_dataclass(annotation)


def _is_hashable(annotation: object) -> bool:
    try:
        hash(annotation)
    except TypeError:
        return False
    return True


def _model_schema(model: type, enclosing: tuple[type, ...]) -> Schema:
    if model in enclosing:
        return Schema.reference(model.__name__)
    try:
        hints = typing.get_type_hints(model, include_extras=True)
    except NameError as error:
        raise RouteError(f"model `{model.__name__}`: {error}") from error
    enclosing = (*enclosing, model)
    fields = [
        field_of(field.name, hints[field.name], _field_default(field), enclosing)
        for field in dataclasses.fields(model)
        if field.init
    ]
    return Schema.model(model.__name__, fields)


def _field_default(field: dataclasses.Field[object]) -> object:
    if field.default is not dataclasses.MISSING:
        return field.default
    if field.default_factory is not dataclasses.MISSING:
        return DEFAULT_FACTORY
    return NO_DEFAULT


def _default_json(default: object) -> str:
    try:
        return json.dumps(default, allow_nan=False)
    except (TypeError, ValueError) as error:
        raise RouteError(f"the default {default!r} is not a JSON value") from error
