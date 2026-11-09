import dataclasses
import typing
from typing import cast

from thermite._schema import is_model, split_none
from thermite._thermite import Body, FieldShape, ModelShape, Schema, Shape


# read once per route, as annotations are slow; the core then builds bodies by it
def body_of(argument: str, annotation: object, schema: Schema) -> Body:
    models: list[ModelShape | None] = []
    shape = _shape_of(annotation, models, {})
    return Body(argument, schema, shape, cast(list[ModelShape], models))


def _shape_of(
    annotation: object, models: list[ModelShape | None], indices: dict[type, int]
) -> Shape:
    annotation, _ = split_none(annotation)
    if typing.get_origin(annotation) is list:
        return Shape.list(_shape_of(typing.get_args(annotation)[0], models, indices))
    if is_model(annotation):
        return Shape.model(_model_index(annotation, models, indices))
    return Shape.any()


def _model_index(
    model: type, models: list[ModelShape | None], indices: dict[type, int]
) -> int:
    if model in indices:
        return indices[model]
    # known before its fields are read, for a model that contains itself
    index = indices[model] = len(models)
    models.append(None)
    hints = typing.get_type_hints(model, include_extras=True)
    fields = [
        FieldShape(
            field.name,
            _shape_of(hints[field.name], models, indices),
            field.kw_only is True,
            field.default_factory is not dataclasses.MISSING,
        )
        for field in dataclasses.fields(model)
        if field.init
    ]
    models[index] = ModelShape(model, fields)
    return index
