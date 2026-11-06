import asyncio
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

Decoder = Callable[[Any], object]


@dataclasses.dataclass(frozen=True)
class BodyArgument:
    name: str
    # None when the value is already what the handler takes
    decoder: Decoder | None


class Route:
    def __init__(
        self, handler: Callable[..., object], body: BodyArgument | None, name: str
    ) -> None:
        self._handler = handler
        self._body = body
        self._name = name

    def _call_handler(self, arguments: dict[str, object], body: object) -> object:
        if self._body is not None:
            decoder = self._body.decoder
            arguments[self._body.name] = body if decoder is None else decoder(body)
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


# called on the event loop's thread
class AsyncRoute(Route):
    def __init__(
        self, handler: Callable[..., object], body: BodyArgument | None, name: str
    ) -> None:
        super().__init__(handler, body, name)
        # the event loop only keeps a weak reference to a task
        self._running: set[asyncio.Task[object]] = set()

    def start(
        self, arguments: dict[str, object], body: object, respond: Respond
    ) -> None:
        try:
            coroutine = self._call_handler(arguments, body)
            task = asyncio.get_running_loop().create_task(
                cast(Coroutine[Any, Any, object], coroutine)
            )
        except BaseException as error:
            self._fail(respond, error)
        else:
            self._running.add(task)
            task.add_done_callback(functools.partial(self._finish, respond))

    def _finish(self, respond: Respond, task: asyncio.Task[object]) -> None:
        self._running.discard(task)
        try:
            respond.succeed(_encode(task.result()))
        except BaseException as error:
            self._fail(respond, error)


def is_async(handler: Callable[..., object]) -> bool:
    return inspect.iscoroutinefunction(handler) or inspect.iscoroutinefunction(
        type(handler).__call__
    )


# read once per route, as annotations are slow; the core has validated the values
def decoder_of(annotation: object) -> Decoder | None:
    return _decoder_of(annotation, {})


def _decoder_of(annotation: object, models: dict[type, Decoder]) -> Decoder | None:
    annotation, _ = split_none(annotation)
    if typing.get_origin(annotation) is list:
        return _list_decoder(typing.get_args(annotation)[0], models)
    if is_model(annotation):
        return models.get(annotation) or _model_decoder(annotation, models)
    return None


def _list_decoder(item: object, models: dict[type, Decoder]) -> Decoder | None:
    item_decoder = _decoder_of(item, models)
    if item_decoder is None:
        return None

    def decode(value: list[object] | None) -> object:
        if value is None:
            return None
        return [item_decoder(element) for element in value]

    return decode


def _model_decoder(model: type, models: dict[type, Decoder]) -> Decoder:
    plain: list[str] = []
    nested: list[tuple[str, Decoder]] = []

    def decode(value: dict[str, object] | None) -> object:
        if value is None:
            return None
        arguments = {name: value[name] for name in plain if name in value}
        for name, decoder in nested:
            if name in value:
                arguments[name] = decoder(value[name])
        return model(**arguments)

    # known before its fields are read, for a model that contains itself
    models[model] = decode
    hints = typing.get_type_hints(model, include_extras=True)
    for field in dataclasses.fields(model):
        if field.init:
            decoder = _decoder_of(hints[field.name], models)
            if decoder is None:
                plain.append(field.name)
            else:
                nested.append((field.name, decoder))
    return decode


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
