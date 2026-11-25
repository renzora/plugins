# Runs once per interpreter, before any script.
#
# What lives here is what is easier to say in Python than in Rust: the vector
# types the API hands back, `print` routed into the engine console, the `set`
# shim, and the canvas `on_draw(g)` receives. Natives come from the `renzora`
# module, which the plugin built before running this.

import sys
import builtins
import renzora as _rz


class vec2(tuple):
    """A 2D vector: a tuple of two floats that also reads as `.x` / `.y`.

    A tuple so it unpacks (`x, y = v`) and compares with plain tuples, and so
    any API function taking a vector accepts `(1, 2)` as readily as `vec2(1, 2)`.
    `+`, `-`, `*` (by a number) and `/` are vector arithmetic, not tuple
    concatenation.
    """

    __slots__ = ()

    def __new__(cls, x=0.0, y=0.0):
        return tuple.__new__(cls, (float(x), float(y)))

    @property
    def x(self):
        return self[0]

    @property
    def y(self):
        return self[1]

    def __add__(self, o):
        return vec2(self[0] + o[0], self[1] + o[1])

    def __radd__(self, o):
        return vec2(o[0] + self[0], o[1] + self[1])

    def __sub__(self, o):
        return vec2(self[0] - o[0], self[1] - o[1])

    def __rsub__(self, o):
        return vec2(o[0] - self[0], o[1] - self[1])

    def __mul__(self, s):
        return vec2(self[0] * s, self[1] * s)

    __rmul__ = __mul__

    def __truediv__(self, s):
        return vec2(self[0] / s, self[1] / s)

    def __neg__(self):
        return vec2(-self[0], -self[1])

    def dot(self, o):
        return self[0] * o[0] + self[1] * o[1]

    def length(self):
        return (self[0] * self[0] + self[1] * self[1]) ** 0.5

    def normalized(self):
        n = self.length()
        return vec2(self[0] / n, self[1] / n) if n > 0 else vec2()

    def __repr__(self):
        return "vec2(%g, %g)" % (self[0], self[1])


class vec3(tuple):
    """A 3D vector: a tuple of three floats that also reads as `.x` / `.y` / `.z`.

    What `get("Transform.translation")` returns, and what every function taking
    a position accepts in place of three numbers: `set_position(v)`.
    """

    __slots__ = ()

    def __new__(cls, x=0.0, y=0.0, z=0.0):
        return tuple.__new__(cls, (float(x), float(y), float(z)))

    @property
    def x(self):
        return self[0]

    @property
    def y(self):
        return self[1]

    @property
    def z(self):
        return self[2]

    def __add__(self, o):
        return vec3(self[0] + o[0], self[1] + o[1], self[2] + o[2])

    def __radd__(self, o):
        return vec3(o[0] + self[0], o[1] + self[1], o[2] + self[2])

    def __sub__(self, o):
        return vec3(self[0] - o[0], self[1] - o[1], self[2] - o[2])

    def __rsub__(self, o):
        return vec3(o[0] - self[0], o[1] - self[1], o[2] - self[2])

    def __mul__(self, s):
        return vec3(self[0] * s, self[1] * s, self[2] * s)

    __rmul__ = __mul__

    def __truediv__(self, s):
        return vec3(self[0] / s, self[1] / s, self[2] / s)

    def __neg__(self):
        return vec3(-self[0], -self[1], -self[2])

    def dot(self, o):
        return self[0] * o[0] + self[1] * o[1] + self[2] * o[2]

    def cross(self, o):
        return vec3(
            self[1] * o[2] - self[2] * o[1],
            self[2] * o[0] - self[0] * o[2],
            self[0] * o[1] - self[1] * o[0],
        )

    def length(self):
        return (self[0] * self[0] + self[1] * self[1] + self[2] * self[2]) ** 0.5

    def normalized(self):
        n = self.length()
        return vec3(self[0] / n, self[1] / n, self[2] / n) if n > 0 else vec3()

    def __repr__(self):
        return "vec3(%g, %g, %g)" % (self[0], self[1], self[2])


class color(tuple):
    """An RGBA colour, channels 0..1: a tuple of four floats that also reads as
    `.r` / `.g` / `.b` / `.a`. Alpha defaults to opaque."""

    __slots__ = ()

    def __new__(cls, r=1.0, g=1.0, b=1.0, a=1.0):
        return tuple.__new__(cls, (float(r), float(g), float(b), float(a)))

    @property
    def r(self):
        return self[0]

    @property
    def g(self):
        return self[1]

    @property
    def b(self):
        return self[2]

    @property
    def a(self):
        return self[3]

    def __repr__(self):
        return "color(%g, %g, %g, %g)" % (self[0], self[1], self[2], self[3])


# -- print() into the engine console -------------------------------------------
#
# There is no terminal to print to in the editor or a shipped game, so stdout
# and stderr are line-buffered into the console: stdout as Info, stderr as
# Error. A partial line (`print(x, end="")`) is held until the line ends, or
# until the hook returns, when the plugin calls `_flush`.


class _ConsoleStream:
    encoding = "utf-8"
    errors = "replace"

    def __init__(self, level):
        self._level = level
        self._buf = ""

    def write(self, text):
        text = str(text)
        self._buf += text
        if "\n" in self._buf:
            *lines, self._buf = self._buf.split("\n")
            for line in lines:
                _rz._log(self._level, line)
        return len(text)

    def flush(self):
        if self._buf:
            _rz._log(self._level, self._buf)
            self._buf = ""

    def isatty(self):
        return False

    def writable(self):
        return True


sys.stdout = _ConsoleStream("Info")
sys.stderr = _ConsoleStream("Error")


def _flush():
    for stream in (sys.stdout, sys.stderr):
        flush = getattr(stream, "flush", None)
        if flush is not None:
            flush()


# -- set ------------------------------------------------------------------------
#
# The engine's reflection write is called `set(path, value)` in every language.
# In Python that name is the builtin set type, which the standard library uses
# everywhere, so the write is NOT put in builtins. Each script instead gets this
# shim in its own globals: called with a "Component.field" path and a value it
# writes the field, and called any other way it is Python's set, including for
# `isinstance(x, set)` and `set[int]`.

_builtin_set = builtins.set


class _SetMeta(type):
    def __call__(cls, *args, **kwargs):
        if len(args) == 2 and not kwargs and isinstance(args[0], str):
            return _rz.set(args[0], args[1])
        return _builtin_set(*args, **kwargs)

    def __instancecheck__(cls, obj):
        return isinstance(obj, _builtin_set)

    def __subclasscheck__(cls, sub):
        return issubclass(sub, _builtin_set)

    def __getitem__(cls, item):
        return _builtin_set[item]

    def __repr__(cls):
        return "<class 'set'>"


class set(metaclass=_SetMeta):
    """Python's set, which also writes a component field: set("Light.intensity", 2.0)."""


# -- on_draw(g) -----------------------------------------------------------------


class _Canvas:
    """The `g` that on_draw(g) receives. Every method draws one shape this frame."""

    __slots__ = ("width", "height")

    def __init__(self, width, height):
        self.width = width
        self.height = height

    line = staticmethod(_rz._draw_line)
    arc = staticmethod(_rz._draw_arc)
    circle = staticmethod(_rz._draw_circle)
    rect = staticmethod(_rz._draw_rect)
    text = staticmethod(_rz._draw_text)
    triangle = staticmethod(_rz._draw_triangle)
    poly = staticmethod(_rz._draw_poly)
