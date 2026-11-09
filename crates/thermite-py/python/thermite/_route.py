import asyncio
import dataclasses
import functools
import inspect
import json
import logging
from collections.abc import Callable, Coroutine
from typing import Any, cast

from thermite._errors import ResponseValidationError
from thermite._schema import is_model
from thermite._thermite import Respond, Schema

_logger = logging.getLogger("thermite.error")


class Route:
    def __init__(
        self, handler: Callable[..., object], name: str, response: Schema | None
    ) -> None:
        self._handler = handler
        self._name = name
        self._response = response

    # also called from Rust, when a model rejects the body before the handler starts
    def fail(self, respond: Respond, error: BaseException) -> None:
        _logger.error("Exception in handler `%s`", self._name, exc_info=error)
        respond.fail(error)

    # what the handler returned, validated and filtered by its return annotation
    def answer(self, value: object) -> bytes:
        body = _encode(value)
        if self._response is None:
            return body
        validated = self._response.validate_response(body)
        if isinstance(validated, list):
            raise ResponseValidationError(validated)
        return validated


# called on a thread of the blocking pool, so the handler may block
class SyncRoute(Route):
    def start(self, arguments: dict[str, object], respond: Respond) -> None:
        try:
            respond.succeed(self.answer(self._handler(**arguments)))
        except BaseException as error:
            self.fail(respond, error)


# called on the event loop's thread
class AsyncRoute(Route):
    def __init__(
        self, handler: Callable[..., object], name: str, response: Schema | None
    ) -> None:
        super().__init__(handler, name, response)
        # the event loop only keeps a weak reference to a task
        self._running: set[asyncio.Task[object]] = set()

    def start(self, arguments: dict[str, object], respond: Respond) -> None:
        try:
            coroutine = self._handler(**arguments)
            task = asyncio.get_running_loop().create_task(
                cast(Coroutine[Any, Any, object], coroutine)
            )
        except BaseException as error:
            self.fail(respond, error)
        else:
            self._running.add(task)
            task.add_done_callback(functools.partial(self._finish, respond))

    def _finish(self, respond: Respond, task: asyncio.Task[object]) -> None:
        self._running.discard(task)
        try:
            respond.succeed(self.answer(task.result()))
        except BaseException as error:
            self.fail(respond, error)


def is_async(handler: Callable[..., object]) -> bool:
    return inspect.iscoroutinefunction(handler) or inspect.iscoroutinefunction(
        type(handler).__call__
    )


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
