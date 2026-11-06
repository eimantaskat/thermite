import asyncio
import json
import signal
from collections.abc import Callable
from types import FrameType
from typing import TypeVar

from thermite._route import AsyncRoute, EventLoopSlot, SyncRoute, is_async
from thermite._signature import describe_handler
from thermite._thermite import CoreApp, TestServer

Handler = TypeVar("Handler", bound=Callable[..., object])


class App:
    def __init__(self) -> None:
        self._core = CoreApp()
        self._event_loop = EventLoopSlot()

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
        document: dict[str, object] = json.loads(self._core.openapi_json())
        return document

    def run(self, host: str = "127.0.0.1", port: int = 8000) -> None:
        asyncio.run(self._serve(host, port))

    async def _serve(self, host: str, port: int) -> None:
        loop = asyncio.get_running_loop()
        self._event_loop.loop = loop
        server = self._core.server(host, port)

        def request_shutdown(_signal: int, _frame: FrameType | None) -> None:
            server.request_shutdown()

        previous = {
            handled: signal.signal(handled, request_shutdown)
            for handled in (signal.SIGINT, signal.SIGTERM)
        }
        try:
            serving = loop.run_in_executor(None, server.serve)
            # a signal handler runs once this thread wakes, which not all signals cause
            while not serving.done():
                await asyncio.wait([serving], timeout=0.1)
            await serving
        finally:
            for handled, handler in previous.items():
                signal.signal(handled, handler)

    def _start_test_server(self, loop: asyncio.AbstractEventLoop) -> TestServer:
        self._event_loop.loop = loop
        return self._core.test_server()

    def _route(self, method: str, path: str) -> Callable[[Handler], Handler]:
        def register(handler: Handler) -> Handler:
            described = describe_handler(handler, path)
            route = (
                AsyncRoute(handler, described.body, described.name, self._event_loop)
                if is_async(handler)
                else SyncRoute(handler, described.body, described.name)
            )
            self._core.add_route(
                method,
                path,
                described.name,
                described.parameters,
                described.body_schema,
                described.response,
                route,
            )
            return handler

        return register
