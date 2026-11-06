import asyncio
import functools
import json as json_module
import string
import threading
import types
import urllib.parse
from collections.abc import Mapping
from typing import Self, TypedDict, Unpack

from thermite._app import App


class RequestOptions(TypedDict, total=False):
    params: Mapping[str, object] | None
    headers: Mapping[str, str] | None
    cookies: Mapping[str, str] | None
    json: object
    content: bytes


class Response:
    def __init__(
        self, status_code: int, headers: dict[str, str], content: bytes
    ) -> None:
        self.status_code = status_code
        self.headers = headers
        self.content = content

    @property
    def text(self) -> str:
        return self.content.decode()

    def json(self) -> object:
        return json_module.loads(self.content)


class TestClient:
    __test__ = False

    def __init__(self, app: App, raise_server_exceptions: bool = True) -> None:
        self._server = app._start_test_server(_event_loop())
        self._raise_server_exceptions = raise_server_exceptions

    def __enter__(self) -> Self:
        return self

    def __exit__(
        self,
        error_type: type[BaseException] | None,
        error: BaseException | None,
        traceback: types.TracebackType | None,
    ) -> None:
        pass

    def request(
        self,
        method: str,
        url: str,
        *,
        params: Mapping[str, object] | None = None,
        headers: Mapping[str, str] | None = None,
        cookies: Mapping[str, str] | None = None,
        json: object = None,
        content: bytes = b"",
    ) -> Response:
        target = urllib.parse.quote(url, safe=string.punctuation)
        if params:
            target += "?" + urllib.parse.urlencode(params, doseq=True)
        sent_headers = list((headers or {}).items())
        if cookies:
            cookie = "; ".join(f"{name}={value}" for name, value in cookies.items())
            sent_headers.append(("cookie", cookie))
        if json is not None:
            content = json_module.dumps(json).encode()
            sent_headers.append(("content-type", "application/json"))
        status, received_headers, body, error = self._server.send(
            method, target, sent_headers, content
        )
        if error is not None and self._raise_server_exceptions:
            raise error
        return Response(status, _join_repeated(received_headers), body)

    def get(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("GET", url, **options)

    def post(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("POST", url, **options)

    def put(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("PUT", url, **options)

    def delete(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("DELETE", url, **options)

    def patch(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("PATCH", url, **options)

    def head(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("HEAD", url, **options)

    def options(self, url: str, **options: Unpack[RequestOptions]) -> Response:
        return self.request("OPTIONS", url, **options)


def _join_repeated(headers: list[tuple[str, str]]) -> dict[str, str]:
    joined: dict[str, str] = {}
    for name, value in headers:
        joined[name] = f"{joined[name]}, {value}" if name in joined else value
    return joined


# the caller waits for the answer, so async handlers need a loop on another thread
@functools.cache
def _event_loop() -> asyncio.AbstractEventLoop:
    loop = asyncio.new_event_loop()
    threading.Thread(target=loop.run_forever, daemon=True).start()
    return loop
