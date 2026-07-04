"""Shared helper: resolve TabPrinter::append_option_line option keys.

Orca's `build_kinematics_page` (the "Motion ability" page) lays out the
`machine_max_*` families with

    append_option_line(optgroup, speed_axis, ...)             // loop variable
    append_option_line(optgroup, "machine_max_jerk_" + axis)  // string concat
    append_option_line(optgroup, "machine_max_acceleration_travel", ...)

The key is built from a loop variable or string concat, so it never appears
as a string literal the plain `append_single_option_line("KEY")` /
`get_option("KEY")` regexes catch. Both Tab.cpp scrapers (page layout and
display order) miss the whole family and fall back — the page scraper to
libslic3r's "Machine limits" category (splitting them off the Motion ability
page), the display-order scraper to "no position" (sorting them last).

`make_append_tracker()` returns a stateful `feed(line) -> list[str]` that
tracks `std::vector<std::string>` literals + `for (… : vec)` bindings and
resolves each `append_option_line` call to its concrete key(s), in source
order. Both scrapers feed every line through it and assign the results the
same way they assign their literal-key matches.
"""
from __future__ import annotations

import re
from typing import Callable

_QUOTED_KEY = re.compile(r'"([a-z][a-z0-9_]*)"')
# `const std::vector<std::string> axes{ "x", "y", "z", "e" };` — may span lines.
_VEC_DECL = re.compile(r"std::vector<std::string>\s+(\w+)\s*\{(.*)$")
# `for (const std::string &axis : axes)` — binds a loop var to a vector.
_LOOP_BIND = re.compile(r"for\s*\(\s*const\s+std::string\s*&\s*(\w+)\s*:\s*(\w+)")
# The key expression (2nd arg) of an append_option_line call, up to the
# label-path arg. First arg must be a bare identifier (the optgroup) so the
# method *definition* (`(ConfigOptionsGroupShp optgroup, …`) doesn't match.
_APPEND = re.compile(r"append_option_line\(\s*\w+\s*,\s*([^,]+?)\s*[,)]")

_LIT = re.compile(r'^"([a-z][a-z0-9_]*)"$')
_CONCAT = re.compile(r'^"([a-z0-9_]*)"\s*\+\s*(\w+)$')
_BARE = re.compile(r"^(\w+)$")


def make_append_tracker() -> Callable[[str], list[str]]:
    vec_literals: dict[str, list[str]] = {}
    loop_bind: dict[str, str] = {}
    pending: list[str | None] = [None]  # name of a vector still collecting keys

    def resolve(expr: str) -> list[str]:
        m = _LIT.match(expr)
        if m:
            return [m.group(1)]
        m = _CONCAT.match(expr)
        if m:
            prefix, var = m.group(1), m.group(2)
            return [prefix + a for a in vec_literals.get(loop_bind.get(var, ""), [])]
        m = _BARE.match(expr)
        if m:
            return list(vec_literals.get(loop_bind.get(m.group(1), ""), []))
        return []

    def feed(line: str) -> list[str]:
        if pending[0] is not None:
            vec_literals[pending[0]].extend(_QUOTED_KEY.findall(line))
            if "}" in line:
                pending[0] = None
        else:
            vd = _VEC_DECL.search(line)
            if vd:
                name, rest = vd.group(1), vd.group(2)
                vec_literals[name] = _QUOTED_KEY.findall(rest)
                if "}" not in rest:
                    pending[0] = name
        lb = _LOOP_BIND.search(line)
        if lb:
            loop_bind[lb.group(1)] = lb.group(2)
        keys: list[str] = []
        for m in _APPEND.finditer(line):
            keys.extend(resolve(m.group(1).strip()))
        return keys

    return feed


if __name__ == "__main__":
    # Self-check against the exact Motion-ability layout shapes.
    feed = make_append_tracker()
    lines = [
        'const std::vector<std::string> speed_axes{',
        '    "machine_max_speed_x",',
        '    "machine_max_speed_e"',
        "};",
        "for (const std::string &speed_axis : speed_axes) {",
        '  append_option_line(optgroup, speed_axis, "path");',
        "}",
        'const std::vector<std::string> axes{ "x", "y" };',
        "for (const std::string &axis : axes) {",
        '  append_option_line(optgroup, "machine_max_jerk_" + axis, "path");',
        "}",
        'append_option_line(optgroup, "machine_max_acceleration_travel", "path");',
    ]
    got = [k for ln in lines for k in feed(ln)]
    assert got == [
        "machine_max_speed_x",
        "machine_max_speed_e",
        "machine_max_jerk_x",
        "machine_max_jerk_y",
        "machine_max_acceleration_travel",
    ], got
    # The method definition line must not be read as a call.
    assert make_append_tracker()(
        "void TabPrinter::append_option_line(ConfigOptionsGroupShp optgroup, "
        "const std::string opt_key, const std::string& label_path)"
    ) == []
    print("ok")
