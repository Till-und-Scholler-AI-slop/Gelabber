"""GitHub Actions expressions (`${{ … }}`, `if:`) evaluated for the tests.

The grammar of the docs (docs.github.com, "Evaluate expressions in
workflows and actions"): literals, context paths, `!`, `==`, `!=`, `<`,
`<=`, `>`, `>=`, `&&`, `||`, parentheses and function calls, with GitHub's
loose equality (strings compared without case, other types as numbers) and
`&&`/`||` returning an operand. What it cannot read raises ValueError, so
a test never decides on an expression it misread.
"""
import math
import re

TOKEN = re.compile(r"""\s*(?:
    (?P<string>'(?:[^']|'')*')
  | (?P<number>-?\d+(?:\.\d+)?)
  | (?P<op>&&|\|\||==|!=|<=|>=|<|>|!|\(|\)|,|\[|\])
  | (?P<name>[A-Za-z_][\w-]*(?:\.(?:[A-Za-z_][\w-]*|\*))*)
)""", re.X)


def tokens(text):
    found, index = [], 0
    text = text.strip()
    while index < len(text):
        match = TOKEN.match(text, index)
        if not match or match.end() == index:
            raise ValueError(f'cannot read {text[index:]!r} in {text!r}')
        kind = match.lastgroup
        found.append((kind, match.group(kind)))
        index = match.end()
    return found


def strip(text):
    """An `if:` or value without its `${{ }}`; a value with text around
    the braces is not an expression this module reads."""
    text = str(text).strip()
    if text.startswith('${{') and text.endswith('}}') and text.count('${{') == 1:
        return text[3:-2].strip()
    if '${{' in text:
        raise ValueError(f'not a single expression: {text!r}')
    return text


def number(value):
    if value is None:
        return 0.0
    if isinstance(value, bool):
        return float(value)
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        if value.strip() == '':
            return 0.0
        try:
            return float(value, 0) if value.lower().startswith('0x') else float(value)
        except ValueError:
            return math.nan
    return math.nan


def truthy(value):
    if isinstance(value, float) and math.isnan(value):
        return False
    return value not in (None, False, 0, '') or isinstance(value, (dict, list))


def equal(left, right):
    if isinstance(left, str) and isinstance(right, str):
        return left.casefold() == right.casefold()
    if type(left) is type(right) and not isinstance(left, (int, float)):
        return left == right
    return number(left) == number(right)


class Parser:
    def __init__(self, text, lookup, functions):
        self.text = text
        self.items = tokens(text)
        self.index = 0
        self.lookup = lookup
        self.functions = functions

    def peek(self):
        return self.items[self.index] if self.index < len(self.items) else (None, None)

    def take(self, value=None):
        kind, text = self.peek()
        if kind is None or (value is not None and text != value):
            raise ValueError(f'expected {value or "more"} in {self.text!r}')
        self.index += 1
        return kind, text

    def parse(self):
        value = self.disjunction()
        if self.index != len(self.items):
            raise ValueError(f'cannot read {self.peek()[1]!r} in {self.text!r}')
        return value

    def disjunction(self):
        value = self.conjunction()
        while self.peek() == ('op', '||'):
            self.take()
            right = self.conjunction()
            value = value if truthy(value) else right
        return value

    def conjunction(self):
        value = self.comparison()
        while self.peek() == ('op', '&&'):
            self.take()
            right = self.comparison()
            value = right if truthy(value) else value
        return value

    def comparison(self):
        value = self.unary()
        while self.peek()[1] in ('==', '!=', '<', '<=', '>', '>=') and self.peek()[0] == 'op':
            _, op = self.take()
            right = self.unary()
            if op == '==':
                value = equal(value, right)
            elif op == '!=':
                value = not equal(value, right)
            else:
                a, b = number(value), number(right)
                value = {'<': a < b, '<=': a <= b, '>': a > b, '>=': a >= b}[op]
        return value

    def unary(self):
        if self.peek() == ('op', '!'):
            self.take()
            return not truthy(self.unary())
        return self.primary()

    def primary(self):
        kind, text = self.take()
        if kind == 'op' and text == '(':
            value = self.disjunction()
            self.take(')')
            return value
        if kind == 'string':
            return text[1:-1].replace("''", "'")
        if kind == 'number':
            return float(text)
        if kind == 'name':
            if text in ('true', 'false'):
                return text == 'true'
            if text == 'null':
                return None
            if self.peek() == ('op', '('):
                self.take()
                arguments = []
                while self.peek() != ('op', ')'):
                    arguments.append(self.disjunction())
                    if self.peek() == ('op', ','):
                        self.take()
                self.take(')')
                if text not in self.functions:
                    raise ValueError(f'unknown function {text}() in {self.text!r}')
                return self.functions[text](*arguments)
            return self.lookup(text)
        raise ValueError(f'cannot read {text!r} in {self.text!r}')


def mentions_status(text):
    return re.search(r'\b(?:success|failure|cancelled|always)\s*\(', text) is not None


def evaluate(expression, lookup, functions=None):
    """The value of `expression` (with or without `${{ }}`); `lookup` maps a
    context path such as `inputs.package` to its value."""
    base = {
        'contains': lambda haystack, needle: (
            any(equal(item, needle) for item in haystack) if isinstance(haystack, list)
            else str(needle).casefold() in str(haystack).casefold()),
        'startsWith': lambda text, prefix: str(text).casefold().startswith(str(prefix).casefold()),
        'endsWith': lambda text, suffix: str(text).casefold().endswith(str(suffix).casefold()),
        'format': lambda text, *values: re.sub(r'\{(\d+)\}', lambda m: str(values[int(m.group(1))]), text),
    }
    return Parser(strip(expression), lookup, {**base, **(functions or {})}).parse()


def condition(expression, lookup, status_ok=True, functions=None):
    """Whether an `if:` holds. Without a status function GitHub adds
    `success()`; `status_ok` is what success() returns here."""
    if expression is None:
        return status_ok
    if isinstance(expression, bool):
        return expression and status_ok
    text = strip(expression)
    status = {
        'success': lambda: status_ok,
        'failure': lambda: not status_ok,
        'cancelled': lambda: False,
        'always': lambda: True,
    }
    value = truthy(evaluate(text, lookup, {**status, **(functions or {})}))
    return value if mentions_status(text) else value and status_ok
