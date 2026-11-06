import json
from collections.abc import Callable
from typing import TypeVar

from thermite._signature import describe_handler
from thermite._thermite import CoreApp

Handler = TypeVar("Handler", bound=Callable[..., object])


class App:
    def __init__(self) -> None:
        self._core = CoreApp()

    def get(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("GET", path)

    def post(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("POST", path)

    def put(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("PUT", path)

    def delete(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("DELETE", path)

    def patch(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("PATCH", path)

    def head(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("HEAD", path)

    def options(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("OPTIONS", path)

    def trace(self, path: str) -> Callable[[Handler], Handler]:
        return self._route("TRACE", path)

    def openapi(self) -> dict[str, object]:
        return json.loads(self._core.openapi_json())

    def _route(self, method: str, path: str) -> Callable[[Handler], Handler]:
        def register(handler: Handler) -> Handler:
            described = describe_handler(handler, path)
            self._core.add_route(
                method,
                path,
                described.name,
                described.parameters,
                described.body,
                described.response,
            )
            return handler

        return register
