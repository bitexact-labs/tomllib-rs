"""Drop-in replacement for the standard library `tomllib` with a Rust parser.

Same results, same TOMLDecodeError (it *is* tomllib.TOMLDecodeError), same messages and positions.
A custom `parse_float` is handled by the standard library itself, so behaviour stays identical."""
import tomllib as _std
from tomllib import TOMLDecodeError

from ._core import parse as _parse

__all__ = ("loads", "load", "TOMLDecodeError")


def load(fp, /, *, parse_float=float):
    """Parse TOML from a binary file object."""
    b = fp.read()
    try:
        s = b.decode()
    except AttributeError:
        raise TypeError(
            "File must be opened in binary mode, e.g. use `open('foo.toml', 'rb')`"
        ) from None
    return loads(s, parse_float=parse_float)


def loads(s, /, *, parse_float=float):
    """Parse TOML from a string."""
    if parse_float is not float:
        return _std.loads(s, parse_float=parse_float)
    try:
        src = s.replace("\r\n", "\n")
    except (AttributeError, TypeError):
        raise TypeError(f"Expected str object, not '{type(s).__qualname__}'") from None
    return _parse(src)
