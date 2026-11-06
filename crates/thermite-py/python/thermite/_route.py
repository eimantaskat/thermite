import asyncio
import concurrent.futures
import dataclasses
import functools
import inspect
import json
import logging
import typing
from collections.abc import Callable, Coroutine
from typing import Any, cast

from thermite._schema import is_model, split_none
from thermite._thermite import Respond

_logger = logging.getLogger("thermite.error")


@dataclasses.dataclass(frozen=True)
class BodyArgument:
    name: str
    annotation: object


# set before the first request, by whatever is about to serve the app
class EventLoopSlot:
    loop: asyncio.AbstractEventLoop


class Route:
    def __init__(
        self, handler: Callable[..., object], body: BodyArgument | None, name: str
    ) -> None:
        self._handler = handler
        self._body = body
        self._name = name

    def _call_handler(self, arguments: dict[str, object], body: object) -> object:
        if self._body is not None:
            arguments[self._body.name] = _decode(self._body.annotation, body)
        return self._handler(**arguments)

    def _fail(self, respond: Respond, error: BaseException) -> None:
        _logger.error("Exception in handler `%s`", self._name, exc_info=error)
        respond.fail(error)


# called on a thread of the blocking pool, so the handler may block
class SyncRoute(Route):
    def start(
        self, arguments: dict[str, object], body: object, respond: Respond
    ) -> None:
        try:
            respond.succeed(_encode(self._call_handler(arguments, body)))
        except BaseException as error:
            self._fail(respond, error)


class AsyncRoute(Route):
    def __init__(
        self,
        handler: Callable[..., object],
        body: BodyArgument | None,
        name: str,
        event_loop: EventLoopSlot,
    ) -> None:
        super().__init__(handler, body, name)
        self._event_loop = event_loop
        # the event loop only keeps a weak reference, and each of these holds its task
        self._running: set[concurrent.futures.Future[object]] = set()

    def start(
        self, arguments: dict[str, object], body: object, respond: Respond
    ) -> None:
        try:
            coroutine = self._call_handler(arguments, body)
            future = asyncio.run_coroutine_threadsafe(
                cast(Coroutine[Any, Any, object], coroutine), self._event_loop.loop
            )
        except BaseException as error:
            self._fail(respond, error)
        else:
            self._running.add(future)
            future.add_done_callback(functools.partial(self._finish, respond))

    # runs on the event loop's thread, once the handler is done
    def _finish(
        self, respond: Respond, future: concurrent.futures.Future[object]
    ) -> None:
        self._running.discard(future)
        try:
            respond.succeed(_encode(future.result()))
        except BaseException as error:
            self._fail(respond, error)


def is_async(handler: Callable[..., object]) -> bool:
    return inspect.iscoroutinefunction(handler) or inspect.iscoroutinefunction(
        type(handler).__call__
    )


def _decode(annotation: object, value: object) -> object:
    annotation, _ = split_none(annotation)
    if value is None:
        return None
    if typing.get_origin(annotation) is list and isinstance(value, list):
        item = typing.get_args(annotation)[0]
        return [_decode(item, element) for element in value]
    if is_model(annotation) and isinstance(value, dict):
        return annotation(
            **{
                name: _decode(field_annotation, value[name])
                for name, field_annotation in _model_fields(annotation)
                if name in value
            }
        )
    return value


@functools.cache
def _model_fields(model: type) -> list[tuple[str, object]]:
    hints = typing.get_type_hints(model, include_extras=True)
    return [
        (field.name, hints[field.name])
        for field in dataclasses.fields(model)
        if field.init
    ]


def _encode(value: object) -> bytes:
    return json.dumps(
        value,
        default=_encode_other,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    ).encode()


def _encode_other(value: object) -> object:
    kind: type = type(value)
    if is_model(kind):
        return {
            field.name: getattr(value, field.name) for field in dataclasses.fields(kind)
        }
    raise TypeError(f"{type(value).__name__} cannot be turned into JSON")
