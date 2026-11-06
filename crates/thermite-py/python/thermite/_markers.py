from typing import ClassVar

from thermite._thermite import ParameterLocation


class Marker:
    location: ClassVar[ParameterLocation | None] = None


class Path(Marker):
    location = ParameterLocation.Path


class Query(Marker):
    location = ParameterLocation.Query


class Header(Marker):
    location = ParameterLocation.Header


class Cookie(Marker):
    location = ParameterLocation.Cookie


class Body(Marker):
    pass


def is_marker(value: object) -> bool:
    return isinstance(value, Marker) or (
        isinstance(value, type) and issubclass(value, Marker)
    )
