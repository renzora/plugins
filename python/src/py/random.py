"""Random numbers.

The Python standard `random` module's everyday functions, under the same names
and with the same arguments, on the plugin's own generator (xorshift64*). The
standard module itself cannot load here; see the plugin's README.

Not cryptographic, as the standard module is not either.
"""

from renzora import _rand, _seed
import math as _math


def seed(a=None):
    """Restart the sequence. The same seed gives the same numbers on every run;
    `None` seeds from the clock."""
    _seed(a)


def random():
    """A float in [0.0, 1.0)."""
    return _rand()


def uniform(a, b):
    """A float between a and b."""
    return a + (b - a) * _rand()


def randrange(start, stop=None, step=1):
    """A random item from range(start, stop, step)."""
    if stop is None:
        start, stop = 0, start
    if step == 0:
        raise ValueError("zero step for randrange()")
    if step > 0:
        n = (stop - start + step - 1) // step
    else:
        n = (stop - start + step + 1) // step
    if n <= 0:
        raise ValueError("empty range for randrange()")
    return start + step * int(_rand() * n)


def randint(a, b):
    """An int N with a <= N <= b."""
    return randrange(a, b + 1)


def choice(seq):
    """One item of a non-empty sequence."""
    if not len(seq):
        raise IndexError("Cannot choose from an empty sequence")
    return seq[int(_rand() * len(seq))]


def choices(population, weights=None, *, cum_weights=None, k=1):
    """k items, with replacement, optionally weighted."""
    n = len(population)
    if cum_weights is None:
        if weights is None:
            return [population[int(_rand() * n)] for _ in range(k)]
        cum_weights = []
        total = 0.0
        for w in weights:
            total += w
            cum_weights.append(total)
    elif weights is not None:
        raise TypeError("Cannot specify both weights and cumulative weights")
    if len(cum_weights) != n:
        raise ValueError("The number of weights does not match the population")
    total = cum_weights[-1] if cum_weights else 0.0
    if total <= 0.0:
        raise ValueError("Total of weights must be greater than zero")
    out = []
    for _ in range(k):
        x = _rand() * total
        lo, hi = 0, n - 1
        while lo < hi:
            mid = (lo + hi) // 2
            if cum_weights[mid] <= x:
                lo = mid + 1
            else:
                hi = mid
        out.append(population[lo])
    return out


def shuffle(x):
    """Shuffle a list in place."""
    for i in range(len(x) - 1, 0, -1):
        j = int(_rand() * (i + 1))
        x[i], x[j] = x[j], x[i]


def sample(population, k):
    """k distinct items, as a new list."""
    pool = list(population)
    if not 0 <= k <= len(pool):
        raise ValueError("Sample larger than population or is negative")
    for i in range(k):
        j = i + int(_rand() * (len(pool) - i))
        pool[i], pool[j] = pool[j], pool[i]
    return pool[:k]


def gauss(mu=0.0, sigma=1.0):
    """A normally distributed float (Box-Muller)."""
    u = 1.0 - _rand()
    v = _rand()
    return mu + sigma * _math.sqrt(-2.0 * _math.log(u)) * _math.cos(_math.tau * v)


normalvariate = gauss


def triangular(low=0.0, high=1.0, mode=None):
    """A float between low and high, most likely near mode."""
    u = _rand()
    c = 0.5 if mode is None else (mode - low) / (high - low)
    if u > c:
        u, c, low, high = 1.0 - u, 1.0 - c, high, low
    return low + (high - low) * _math.sqrt(u * c)


def getrandbits(k):
    """An int with k random bits."""
    if k < 0:
        raise ValueError("number of bits must be non-negative")
    out = 0
    for _ in range((k + 31) // 32):
        out = (out << 32) | int(_rand() * 4294967296.0)
    return out >> ((k + 31) // 32 * 32 - k)
