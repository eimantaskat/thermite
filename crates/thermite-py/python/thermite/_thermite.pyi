from typing import ClassVar, final

__all__ = [
    "CoreApp",
    "Field",
    "Parameter",
    "ParameterLocation",
    "Respond",
    "RouteError",
    "Schema",
    "Server",
    "TestServer",
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
    def __new__(
        cls, argument: str, location: ParameterLocation, field: Field
    ) -> Parameter: ...

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
        route: object,
        on_event_loop: bool,
    ) -> None: ...
    def server(self, host: str, port: int, event_loop: object) -> Server: ...
    def test_server(self, event_loop: object) -> TestServer: ...
    def openapi_json(self) -> str: ...

@final
class Respond:
    def succeed(self, body: bytes) -> None: ...
    def fail(self, error: BaseException) -> None: ...

@final
class Server:
    def serve(self) -> None: ...
    def request_shutdown(self) -> None: ...

@final
class TestServer:
    def send(
        self,
        method: str,
        target: str,
        headers: list[tuple[str, str]],
        body: bytes,
    ) -> tuple[int, list[tuple[str, str]], bytes, BaseException | None]: ...
