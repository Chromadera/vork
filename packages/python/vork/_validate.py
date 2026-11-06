"""Helper option diagnostics with the CLI's spelling and display rules."""
from functools import wraps
from inspect import Parameter, signature
import json


def _distance(a, b):
    d = [[i if j == 0 else j if i == 0 else 0 for j in range(len(b) + 1)]
         for i in range(len(a) + 1)]
    for i in range(1, len(a) + 1):
        for j in range(1, len(b) + 1):
            d[i][j] = min(d[i-1][j] + 1, d[i][j-1] + 1, d[i-1][j-1] + (a[i-1] != b[j-1]))
            if i > 1 and j > 1 and a[i-1] == b[j-2] and a[i-2] == b[j-1]:
                d[i][j] = min(d[i][j], d[i-2][j-2] + 1)
    return d[-1][-1]


def _check_options(value, allowed):
    for key in value:
        if key in allowed:
            continue
        valid = sorted(set(allowed), key=lambda name: (_distance(key, name), name))
        suggestion = ''
        if valid and _distance(key, valid[0]) <= max(1, len(key) // 3):
            suggestion = f' did you mean {json.dumps(valid[0], ensure_ascii=False)}?'
        label = f'Nearest valid names (5 of {len(valid)})' if len(valid) > 5 else 'Valid names'
        names = f'{label}: {", ".join(valid[:5])}' if valid else 'no valid names are declared'
        raise TypeError(f'unknown option {json.dumps(key, ensure_ascii=False)};{suggestion} {names}')


def helper_options(function):
    """Diagnose unknown keywords before Python's argument binding rejects them."""
    parameters = signature(function).parameters
    allowed = [name for name, parameter in parameters.items()
               if parameter.kind in (Parameter.POSITIONAL_OR_KEYWORD, Parameter.KEYWORD_ONLY)]

    @wraps(function)
    def checked(*args, **kwargs):
        _check_options(kwargs, allowed)
        return function(*args, **kwargs)
    return checked
