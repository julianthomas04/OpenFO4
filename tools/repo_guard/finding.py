from dataclasses import dataclass


@dataclass(frozen=True)
class Finding:
    rule: str
    path: str
    detail: str
    line: int | None = None

    def location(self) -> str:
        return f"{self.path}:{self.line}" if self.line is not None else self.path
