from typing import ClassVar, final

__all__ = [
    "CoreApp",
    "Field",
    "Parameter",
    "ParameterLocation",
    "RouteError",
    "Schema",
]

class RouteError(Exception): ...

@final
class Schema:
    @staticmethod
    def string() -> Schema: ...
    @staticmethod
    def integer() -> Schema: ...
    @staticmethod
    def number() -> Schema: ...
    @staticmethod
    def boolean() -> Schema: ...
    @staticmethod
    def any() -> Schema: ...
    @staticmethod
    def array(items: Schema) -> Schema: ...
    @staticmethod
    def model(name: str, fields: list[Field]) -> Schema: ...
    @staticmethod
    def reference(name: str) -> Schema: ...

@final
class Field:
    @staticmethod
    def required(name: str, schema: Schema) -> Field: ...
    @staticmethod
    def optional(name: str, schema: Schema) -> Field: ...
    @staticmethod
    def with_default(name: str, schema: Schema, default_json: str) -> Field: ...

@final
class ParameterLocation:
    Path: ClassVar[ParameterLocation]
    Query: ClassVar[ParameterLocation]
    Header: ClassVar[ParameterLocation]
    Cookie: ClassVar[ParameterLocation]

@final
class Parameter:
    def __new__(cls, location: ParameterLocation, field: Field) -> Parameter: ...

@final
class CoreApp:
    def __new__(cls) -> CoreApp: ...
    def add_route(
        self,
        method: str,
        path: str,
        name: str,
        parameters: list[Parameter],
        body: Schema | None,
        response: Schema | None,
    ) -> None: ...
    def openapi_json(self) -> str: ...
