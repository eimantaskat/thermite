class ResponseValidationError(Exception):
    def __init__(self, errors: list[dict[str, object]]) -> None:
        super().__init__(errors)
        self._errors = errors

    def errors(self) -> list[dict[str, object]]:
        return self._errors

    # the log of a failed handler is the only place these errors show
    def __str__(self) -> str:
        count = len(self._errors)
        plural = "" if count == 1 else "s"
        lines = [f"{count} validation error{plural} in the response"]
        lines.extend(f"  {error}" for error in self._errors)
        return "\n".join(lines)
