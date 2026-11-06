import dataclasses
import inspect
import re
import typing
from collections.abc import Callable
from typing import Annotated

from thermite._markers import Marker, is_marker
from thermite._schema import (
    NO_DEFAULT,
    field_of,
    is_model,
    schema_of,
    split_none,
)
from thermite._thermite import Parameter, ParameterLocation, RouteError, Schema

_PATH_PARAMETER = re.compile(r"\{([^{}]*)\}")


@dataclasses.dataclass(frozen=True)
class HandlerDescription:
    name: str
    parameters: list[Parameter]
    body: Schema | None
    response: Schema | None


def describe_handler(handler: Callable[..., object], path: str) -> HandlerDescription:
    name = getattr(handler, "__name__", type(handler).__name__)
    try:
        signature = inspect.signature(handler, eval_str=True)
    except NameError as error:
        raise RouteError(f"handler `{name}`: {error}") from error
    path_names = set(_PATH_PARAMETER.findall(path))
    parameters = []
    bodies = []
    for parameter in signature.parameters.values():
        try:
            described = _describe_parameter(parameter, path_names)
        except RouteError as error:
            raise RouteError(
                f"parameter `{parameter.name}` of `{name}`: {error}"
            ) from error
        if isinstance(described, Parameter):
            parameters.append(described)
        else:
            bodies.append(described)
    if len(bodies) > 1:
        raise RouteError(f"handler `{name}` declares more than one body")
    try:
        response = _describe_response(signature.return_annotation)
    except RouteError as error:
        raise RouteError(f"return type of `{name}`: {error}") from error
    return HandlerDescription(name, parameters, bodies[0] if bodies else None, response)


def _describe_parameter(
    parameter: inspect.Parameter, path_names: set[str]
) -> Parameter | Schema:
    if parameter.kind in (
        parameter.POSITIONAL_ONLY,
        parameter.VAR_POSITIONAL,
        parameter.VAR_KEYWORD,
    ):
        raise RouteError("a handler's parameters must be passable by name")
    if is_marker(parameter.default):
        raise RouteError("write the marker in the type, as `Annotated[str, Query()]`")
    location = _location_of(parameter, path_names)
    if location is None:
        if parameter.default is not NO_DEFAULT:
            raise RouteError("an optional body is not supported yet")
        return schema_of(parameter.annotation)
    name = parameter.name
    if location == ParameterLocation.Header:
        name = name.replace("_", "-")
    return Parameter(location, field_of(name, parameter.annotation, parameter.default))


def _marker_of(annotation: object) -> Marker | type[Marker] | None:
    if typing.get_origin(annotation) is not Annotated:
        return None
    markers = [item for item in typing.get_args(annotation)[1:] if is_marker(item)]
    if len(markers) > 1:
        raise RouteError("only one of Path, Query, Header, Cookie and Body can be used")
    return markers[0] if markers else None


# None is the body; like FastAPI, only a marked list is read from the query
def _location_of(
    parameter: inspect.Parameter, path_names: set[str]
) -> ParameterLocation | None:
    marker = _marker_of(parameter.annotation)
    if marker is not None:
        return marker.location
    if parameter.name in path_names:
        return ParameterLocation.Path
    if _is_scalar(split_none(parameter.annotation)[0]):
        return ParameterLocation.Query
    return None


def _is_scalar(annotation: object) -> bool:
    return (
        annotation is not list
        and typing.get_origin(annotation) is not list
        and not is_model(annotation)
    )


def _describe_response(annotation: object) -> Schema | None:
    if annotation is inspect.Signature.empty or annotation is None:
        return None
    return schema_of(annotation)
