#!/usr/bin/env python3

from __future__ import annotations

import argparse
import re
import subprocess
import sys
import tempfile
from collections import defaultdict, deque
from dataclasses import dataclass, field
from pathlib import Path


CALL_TERMINATOR_PREFIXES = (
    "goto",
    "switchInt",
    "drop",
    "falseEdge",
    "falseUnwind",
    "yield",
    "backedge",
    "return",
    "resume",
    "terminate",
    "unreachable",
    "assert",
)


@dataclass(frozen=True)
class OverrideSpec:
    trait_alias: str
    method: str
    receiver_alias: str


@dataclass
class BasicBlock:
    name: str
    cleanup: bool
    lines: list[tuple[int, str]] = field(default_factory=list)
    successors: set[str] = field(default_factory=set)


@dataclass
class CallSite:
    function_name: str
    block_name: str
    mir_line: int
    text: str
    callee_expr: str
    trait_path: str | None
    receiver_path: str | None
    receiver_is_dyn: bool
    receiver_is_unknown: bool
    method_name: str | None
    key_candidates: list[str]
    override_specs: list[OverrideSpec]
    unwind: str
    unwind_block: str | None
    callee_functions: set[str] = field(default_factory=set)
    has_drop_cleanup: bool = False
    cleanup_drops: list["CleanupDrop"] = field(default_factory=list)


@dataclass
class CleanupDrop:
    block_name: str
    line_no: int
    text: str
    place: str
    root_local: str | None
    debug_names: tuple[str, ...]
    type_name: str | None


@dataclass
class Function:
    name: str
    header: str
    start_line: int
    receiver_type: str | None
    keys: set[str]
    local_types: dict[str, str] = field(default_factory=dict)
    local_debug_names: dict[str, set[str]] = field(default_factory=lambda: defaultdict(set))
    blocks: dict[str, BasicBlock] = field(default_factory=dict)
    calls: list[CallSite] = field(default_factory=list)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Verify that every in-crate Rust frame that may be on the stack when "
            "`out_of_memory` is called has no live drop cleanup across the relevant call edge."
        )
    )
    parser.add_argument(
        "--manifest-path",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "Cargo.toml",
        help="Path to the Cargo.toml for the crate to analyze.",
    )
    parser.add_argument(
        "--cargo",
        default="cargo",
        help="Path to the cargo executable.",
    )
    parser.add_argument(
        "--cargo-arg",
        action="append",
        default=[],
        help="Extra argument passed to `cargo rustc` before `--`.",
    )
    parser.add_argument(
        "--rustc-arg",
        action="append",
        default=[],
        help="Extra argument passed to rustc after `--`.",
    )
    parser.add_argument(
        "--mir",
        type=Path,
        help="Analyze an existing MIR file instead of invoking cargo rustc.",
    )
    parser.add_argument(
        "--target-substring",
        action="append",
        default=["out_of_memory"],
        help="Substring that identifies direct target calls. May be specified multiple times.",
    )
    parser.add_argument(
        "--keep-mir",
        action="store_true",
        help="Keep the generated MIR file on disk.",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        help="Only print failures.",
    )
    return parser.parse_args()


def emit_mir(args: argparse.Namespace) -> Path:
    with tempfile.NamedTemporaryFile(
        prefix="mmtk-oom-stack-", suffix=".mir", delete=False
    ) as handle:
        mir_path = Path(handle.name)

    cmd = [
        args.cargo,
        "rustc",
        "--manifest-path",
        str(args.manifest_path),
        "--lib",
        *args.cargo_arg,
        "--",
        f"--emit=mir={mir_path}",
        *args.rustc_arg,
    ]
    subprocess.run(cmd, check=True)
    return mir_path


def strip_generic_arguments(text: str) -> str:
    text = text.replace("::<", "<")
    out: list[str] = []
    depth = 0
    for ch in text:
        if ch == "<":
            depth += 1
            continue
        if ch == ">":
            depth = max(depth - 1, 0)
            continue
        if depth == 0:
            out.append(ch)
    return "".join(out)


def normalize_whitespace(text: str) -> str:
    return re.sub(r"\s+", " ", text).strip()


def normalize_path(text: str) -> str:
    text = strip_generic_arguments(text.strip())
    text = normalize_whitespace(text)
    text = re.sub(r"\s*::\s*", "::", text)
    text = re.sub(r"\s+as\s+", " as ", text)
    text = re.sub(r"\s+", " ", text)
    text = text.rstrip(":")
    return text


def find_matching_angle(text: str, start: int) -> int | None:
    depth = 0
    for i in range(start, len(text)):
        if text[i] == "<":
            depth += 1
        elif text[i] == ">":
            depth -= 1
            if depth == 0:
                return i
    return None


def normalize_type(text: str) -> str:
    text = normalize_path(text)
    text = re.sub(r"^&(?:mut )?", "", text)
    text = re.sub(r"^\*const ", "", text)
    text = re.sub(r"^\*mut ", "", text)
    return text.strip()


def split_top_level_once(text: str, separator: str) -> tuple[str, str | None]:
    depth = 0
    i = 0
    while i < len(text):
        ch = text[i]
        if ch in "<({[":
            depth += 1
        elif ch in ">)}]":
            depth = max(depth - 1, 0)
        elif depth == 0 and text.startswith(separator, i):
            return text[:i], text[i + len(separator) :]
        i += 1
    return text, None


def split_top_level_keyword(text: str, keyword: str) -> tuple[str, str | None]:
    needle = f" {keyword} "
    depth = 0
    for i, ch in enumerate(text):
        if ch in "<({[":
            depth += 1
        elif ch in ">)}]":
            depth = max(depth - 1, 0)
        elif depth == 0 and text.startswith(needle, i):
            return text[:i], text[i + len(needle) :]
    return text, None


def aliases_for_path(path: str | None) -> set[str]:
    if path is None:
        return set()
    path = normalize_path(path)
    if not path:
        return set()
    aliases = {path}
    if path.startswith("dyn "):
        aliases.add(path[len("dyn ") :])
    parts = path.split("::")
    aliases.add(parts[-1])
    return aliases


def module_suffix(path: str) -> str | None:
    parts = path.split("::")
    if len(parts) < 2:
        return None
    return "::".join(parts[:-1])


def extract_function_header_parts(header: str) -> tuple[str, list[str]]:
    header = header.strip()
    assert header.startswith("fn "), header
    name_end = header.index("(")
    name = header[3:name_end].strip()
    params_blob = header[name_end + 1 : header.rindex(") ->")]
    params: list[str] = []
    depth = 0
    start = 0
    for i, ch in enumerate(params_blob):
        if ch in "<({[":
            depth += 1
        elif ch in ">)}]":
            depth = max(depth - 1, 0)
        elif ch == "," and depth == 0:
            params.append(params_blob[start:i].strip())
            start = i + 1
    tail = params_blob[start:].strip()
    if tail:
        params.append(tail)
    return name, params


def receiver_type_from_params(params: list[str]) -> str | None:
    if not params or ":" not in params[0]:
        return None
    _, receiver_type = params[0].split(":", 1)
    return normalize_type(receiver_type)


def parse_header_locals(params: list[str]) -> dict[str, str]:
    locals_by_id: dict[str, str] = {}
    for param in params:
        if ":" not in param:
            continue
        local_id, type_name = param.split(":", 1)
        local_id = local_id.strip()
        if re.fullmatch(r"_\d+", local_id):
            locals_by_id[local_id] = normalize_whitespace(type_name)
    return locals_by_id


def function_keys(name: str, receiver_type: str | None) -> set[str]:
    cleaned = normalize_path(re.sub(r"::<impl at [^>]+>::", "::", name))
    keys = {cleaned}
    parts = cleaned.split("::")
    if len(parts) >= 2:
        keys.add("::".join(parts[-2:]))

    method = parts[-1]
    if receiver_type and receiver_type != "Self":
        for alias in aliases_for_path(receiver_type):
            keys.add(f"{alias}::{method}")
        if receiver_type.startswith("{closure@"):
            keys.add(receiver_type)

    closure_match = re.search(r"(\{closure@[^}]+\})", name)
    if closure_match:
        keys.add(closure_match.group(1))

    return keys


def split_call_target(expr: str) -> str | None:
    expr = expr.strip()
    angle_depth = 0
    brace_depth = 0
    bracket_depth = 0
    paren_depth = 0
    for i, ch in enumerate(expr):
        if ch == "<":
            angle_depth += 1
        elif ch == ">":
            angle_depth = max(angle_depth - 1, 0)
        elif ch == "{":
            brace_depth += 1
        elif ch == "}":
            brace_depth = max(brace_depth - 1, 0)
        elif ch == "[":
            bracket_depth += 1
        elif ch == "]":
            bracket_depth = max(bracket_depth - 1, 0)
        elif ch == "(":
            if angle_depth == 0 and brace_depth == 0 and bracket_depth == 0 and paren_depth == 0:
                return expr[:i].strip()
            paren_depth += 1
        elif ch == ")":
            paren_depth = max(paren_depth - 1, 0)
    return None


def parse_trait_style_callee(expr: str) -> tuple[str | None, str | None, bool, bool, str | None]:
    expr = normalize_whitespace(expr.strip())
    if not expr.startswith("<"):
        expr = normalize_path(expr)
        method = expr.split("::")[-1] if "::" in expr else None
        return None, None, False, False, method

    end = find_matching_angle(expr, 0)
    if end is None or not expr[end + 1 :].startswith("::"):
        expr = normalize_path(expr)
        method = expr.split("::")[-1] if "::" in expr else None
        return None, None, False, False, method

    inner = expr[1:end]
    method = normalize_path(expr[end + 3 :])
    left, right = split_top_level_once(inner, " as ")
    if right is None:
        receiver = normalize_type(left)
        return None, receiver, receiver.startswith("dyn "), receiver == "Self", method

    receiver = normalize_type(left)
    trait = normalize_path(right)
    return trait, receiver, receiver.startswith("dyn "), receiver == "Self", method


def call_key_candidates(
    callee_expr: str,
) -> tuple[list[str], list[OverrideSpec], str | None, str | None, bool, bool, str | None]:
    candidates: list[str] = []
    seen: set[str] = set()

    def add(key: str | None) -> None:
        if not key:
            return
        if key not in seen:
            seen.add(key)
            candidates.append(key)

    (
        trait_path,
        receiver_path,
        receiver_is_dyn,
        receiver_is_unknown,
        method,
    ) = parse_trait_style_callee(callee_expr)

    if trait_path is not None and receiver_path is not None and method is not None:
        normalized_callee_expr = f"<{receiver_path} as {trait_path}>::{method}"
    elif receiver_path is not None and method is not None and callee_expr.strip().startswith("<"):
        normalized_callee_expr = f"<{receiver_path}>::{method}"
    else:
        normalized_callee_expr = normalize_path(callee_expr)

    add(normalized_callee_expr)
    parts = normalized_callee_expr.split("::")
    if len(parts) >= 2:
        add("::".join(parts[-2:]))

    override_specs: list[OverrideSpec] = []

    if method is not None:
        for alias in aliases_for_path(receiver_path):
            add(f"{alias}::{method}")
        for alias in aliases_for_path(trait_path):
            add(f"{alias}::{method}")
        closure_match = re.search(r"(\{closure@[^}]+\})", normalized_callee_expr)
        if closure_match:
            add(closure_match.group(1))

        if trait_path is not None:
            for trait_alias in aliases_for_path(trait_path):
                if receiver_path is not None and not receiver_is_dyn and not receiver_is_unknown:
                    for receiver_alias in aliases_for_path(receiver_path):
                        override_specs.append(
                            OverrideSpec(
                                trait_alias=trait_alias,
                                method=method,
                                receiver_alias=receiver_alias,
                            )
                        )
                else:
                    override_specs.append(
                        OverrideSpec(
                            trait_alias=trait_alias,
                            method=method,
                            receiver_alias="*",
                        )
                    )

    return (
        candidates,
        override_specs,
        trait_path,
        receiver_path,
        receiver_is_dyn,
        receiver_is_unknown,
        method,
    )


def parse_unwind(text: str) -> tuple[str, str | None]:
    text = text.strip()
    if "unwind continue" in text:
        return "continue", None
    if "unwind terminate(cleanup)" in text:
        return "terminate(cleanup)", None
    match = re.search(r"unwind: (bb\d+)", text)
    if match:
        return "cleanup", match.group(1)
    return "unknown", None


def parse_mir(mir_path: Path) -> dict[str, Function]:
    lines = mir_path.read_text().splitlines()
    functions: dict[str, Function] = {}
    i = 0
    while i < len(lines):
        if not lines[i].startswith("fn "):
            i += 1
            continue

        start = i
        depth = lines[i].count("{") - lines[i].count("}")
        i += 1
        while i < len(lines) and depth > 0:
            depth += lines[i].count("{") - lines[i].count("}")
            i += 1

        function_lines = lines[start:i]
        header = function_lines[0]
        name, params = extract_function_header_parts(header)
        receiver_type = receiver_type_from_params(params)
        func = Function(
            name=name,
            header=header,
            start_line=start + 1,
            receiver_type=receiver_type,
            keys=function_keys(name, receiver_type),
            local_types=parse_header_locals(params),
        )

        current_block: BasicBlock | None = None
        for offset, raw_line in enumerate(function_lines[1:], start=1):
            line_no = start + offset + 1

            let_match = re.match(r"^\s*let(?: mut)? (_\d+): (.+);$", raw_line)
            if let_match:
                func.local_types[let_match.group(1)] = normalize_whitespace(let_match.group(2))

            debug_match = re.match(r"^\s*debug ([^\s]+) => (_\d+);$", raw_line)
            if debug_match:
                func.local_debug_names[debug_match.group(2)].add(debug_match.group(1))

            block_match = re.match(r"^\s*(bb\d+)(?: \(cleanup\))?: \{$", raw_line)
            if block_match:
                block_name = block_match.group(1)
                cleanup = "(cleanup)" in raw_line
                current_block = BasicBlock(name=block_name, cleanup=cleanup)
                func.blocks[block_name] = current_block
                continue

            if current_block is None:
                continue

            current_block.lines.append((line_no, raw_line))
            current_block.successors.update(re.findall(r"\bbb\d+\b", raw_line))

            stripped = raw_line.strip().rstrip(";")
            if "-> [" not in stripped or "[return:" not in stripped:
                continue
            if any(stripped.startswith(prefix) for prefix in CALL_TERMINATOR_PREFIXES):
                continue

            prefix, _, tail = stripped.partition(" -> [")
            if "=" in prefix:
                _, _, expr = prefix.partition("=")
            else:
                expr = prefix
            callee_expr = split_call_target(expr)
            if callee_expr is None:
                continue

            (
                key_candidates,
                override_specs,
                trait_path,
                receiver_path,
                receiver_is_dyn,
                receiver_is_unknown,
                method_name,
            ) = call_key_candidates(callee_expr)
            unwind, unwind_block = parse_unwind(tail.rstrip("]"))

            func.calls.append(
                CallSite(
                    function_name=func.name,
                    block_name=current_block.name,
                    mir_line=line_no,
                    text=stripped,
                    callee_expr=key_candidates[0],
                    trait_path=trait_path,
                    receiver_path=receiver_path,
                    receiver_is_dyn=receiver_is_dyn,
                    receiver_is_unknown=receiver_is_unknown,
                    method_name=method_name,
                    key_candidates=key_candidates,
                    override_specs=override_specs,
                    unwind=unwind,
                    unwind_block=unwind_block,
                )
            )

        functions[func.name] = func

    return functions


def strip_non_code(text: str) -> str:
    out: list[str] = []
    i = 0
    block_comment_depth = 0
    while i < len(text):
        if block_comment_depth:
            if text.startswith("/*", i):
                block_comment_depth += 1
                out.extend("  ")
                i += 2
                continue
            if text.startswith("*/", i):
                block_comment_depth -= 1
                out.extend("  ")
                i += 2
                continue
            out.append("\n" if text[i] == "\n" else " ")
            i += 1
            continue

        if text.startswith("//", i):
            while i < len(text) and text[i] != "\n":
                out.append(" ")
                i += 1
            continue

        if text.startswith("/*", i):
            block_comment_depth = 1
            out.extend("  ")
            i += 2
            continue

        if text[i] == '"':
            out.append(" ")
            i += 1
            while i < len(text):
                ch = text[i]
                out.append("\n" if ch == "\n" else " ")
                i += 1
                if ch == "\\" and i < len(text):
                    out.append("\n" if text[i] == "\n" else " ")
                    i += 1
                    continue
                if ch == '"':
                    break
            continue

        out.append(text[i])
        i += 1

    return "".join(out)


def parse_impl_methods(body: str) -> set[str]:
    methods: set[str] = set()
    depth = 0
    i = 0
    while i < len(body):
        ch = body[i]
        if ch == "{":
            depth += 1
            i += 1
            continue
        if ch == "}":
            depth = max(depth - 1, 0)
            i += 1
            continue
        if depth == 0 and body.startswith("fn ", i):
            i += 3
            start = i
            while i < len(body) and (body[i].isalnum() or body[i] == "_"):
                i += 1
            if start != i:
                methods.add(body[start:i])
            continue
        i += 1
    return methods


def parse_trait_impls(crate_root: Path) -> dict[tuple[str, str], set[str]]:
    overrides: dict[tuple[str, str], set[str]] = defaultdict(set)
    for source_file in sorted(crate_root.glob("src/**/*.rs")):
        text = strip_non_code(source_file.read_text())
        i = 0
        while True:
            match = re.search(r"\bimpl\b", text[i:])
            if match is None:
                break
            start = i + match.start()
            header_start = start + len("impl")
            header_depth = 0
            j = header_start
            while j < len(text):
                ch = text[j]
                if ch in "<([":
                    header_depth += 1
                elif ch in ">)]":
                    header_depth = max(header_depth - 1, 0)
                elif ch == "{" and header_depth == 0:
                    break
                j += 1
            if j >= len(text):
                break

            header = normalize_whitespace(text[start:j])
            body_start = j + 1
            body_depth = 1
            j += 1
            while j < len(text) and body_depth > 0:
                if text[j] == "{":
                    body_depth += 1
                elif text[j] == "}":
                    body_depth -= 1
                j += 1
            body = text[body_start : j - 1]

            impl_tail = header[len("impl") :].strip()
            if impl_tail.startswith("<"):
                generic_depth = 0
                k = 0
                while k < len(impl_tail):
                    if impl_tail[k] == "<":
                        generic_depth += 1
                    elif impl_tail[k] == ">":
                        generic_depth -= 1
                        if generic_depth == 0:
                            k += 1
                            break
                    k += 1
                impl_tail = impl_tail[k:].strip()

            left, right = split_top_level_keyword(impl_tail, "for")
            if right is None:
                i = j
                continue

            right, _ = split_top_level_keyword(right, "where")
            trait_path = normalize_path(left)
            receiver_path = normalize_type(right)
            method_names = parse_impl_methods(body)

            for method_name in method_names:
                for trait_alias in aliases_for_path(trait_path):
                    for receiver_alias in aliases_for_path(receiver_path):
                        overrides[(trait_alias, method_name)].add(receiver_alias)

            i = j

    return overrides


def resolve_calls(
    functions: dict[str, Function],
    overrides: dict[tuple[str, str], set[str]],
) -> dict[str, list[CallSite]]:
    function_keys_to_names: dict[str, set[str]] = defaultdict(set)
    for function in functions.values():
        for key in function.keys:
            function_keys_to_names[key].add(function.name)

    callers_by_callee: dict[str, list[CallSite]] = defaultdict(list)

    for function in functions.values():
        for call in function.calls:
            matches: set[str] = set()
            for key in call.key_candidates:
                matches.update(function_keys_to_names.get(key, set()))

            default_trait_matches: set[str] = set()
            override_matches: set[str] = set()

            if call.method_name is not None:
                for trait_alias in aliases_for_path(call.trait_path):
                    default_trait_matches.update(
                        function_keys_to_names.get(f"{trait_alias}::{call.method_name}", set())
                    )

                    if call.receiver_is_dyn or call.receiver_is_unknown:
                        receiver_aliases = overrides.get((trait_alias, call.method_name), set())
                    else:
                        allowed = aliases_for_path(call.receiver_path)
                        receiver_aliases = {
                            receiver_alias
                            for receiver_alias in overrides.get((trait_alias, call.method_name), set())
                            if receiver_alias in allowed
                        }

                    for receiver_alias in receiver_aliases:
                        override_matches.update(
                            function_keys_to_names.get(
                                f"{receiver_alias}::{call.method_name}", set()
                            )
                        )

            if override_matches:
                matches.update(override_matches)
            elif default_trait_matches:
                matches.update(default_trait_matches)

            call.callee_functions = matches
            for callee_name in matches:
                callers_by_callee[callee_name].append(call)

    return callers_by_callee


def extract_drop_place(text: str) -> str | None:
    text = text.strip()
    if not text.startswith("drop("):
        return None
    depth = 1
    start = len("drop(")
    i = start
    while i < len(text):
        if text[i] == "(":
            depth += 1
        elif text[i] == ")":
            depth -= 1
            if depth == 0:
                return text[start:i].strip()
        i += 1
    return None


def describe_cleanup_drop(function: Function, block_name: str, line_no: int, text: str) -> CleanupDrop:
    place = extract_drop_place(text) or "?"
    root_local_match = re.search(r"\b(_\d+)\b", place)
    root_local = root_local_match.group(1) if root_local_match else None
    debug_names = tuple(sorted(function.local_debug_names.get(root_local, set())))
    type_name = None
    if place in function.local_types:
        type_name = function.local_types[place]
    elif root_local is not None:
        type_name = function.local_types.get(root_local)
    return CleanupDrop(
        block_name=block_name,
        line_no=line_no,
        text=text,
        place=place,
        root_local=root_local,
        debug_names=debug_names,
        type_name=type_name,
    )


def reachable_cleanup_drops(function: Function, start_block: str | None) -> list[CleanupDrop]:
    if start_block is None:
        return []
    found: list[CleanupDrop] = []
    seen: set[str] = set()
    queue = deque([start_block])
    while queue:
        block_name = queue.popleft()
        if block_name in seen:
            continue
        seen.add(block_name)
        block = function.blocks.get(block_name)
        if block is None:
            continue
        for line_no, raw_line in block.lines:
            if raw_line.strip().startswith("drop("):
                found.append(
                    describe_cleanup_drop(function, block_name, line_no, raw_line.strip())
                )
        for successor in block.successors:
            if successor not in seen:
                queue.append(successor)
    return found


def analyze_cleanup(functions: dict[str, Function]) -> None:
    for function in functions.values():
        for call in function.calls:
            drops = reachable_cleanup_drops(function, call.unwind_block)
            call.cleanup_drops = drops
            call.has_drop_cleanup = bool(drops)


def find_target_calls(
    functions: dict[str, Function], target_substrings: list[str]
) -> list[CallSite]:
    matches: list[CallSite] = []
    for function in functions.values():
        for call in function.calls:
            if any(target in call.callee_expr for target in target_substrings):
                matches.append(call)
    return matches


def reverse_reachable_functions(
    target_calls: list[CallSite], callers_by_callee: dict[str, list[CallSite]]
) -> tuple[set[str], list[CallSite]]:
    reachable_functions = {call.function_name for call in target_calls}
    reachable_edges: list[CallSite] = []
    queue = deque(reachable_functions)
    seen_edges: set[tuple[str, int, str]] = set()

    while queue:
        callee_name = queue.popleft()
        for caller_call in callers_by_callee.get(callee_name, []):
            edge_key = (
                caller_call.function_name,
                caller_call.mir_line,
                caller_call.text,
            )
            if edge_key not in seen_edges:
                seen_edges.add(edge_key)
                reachable_edges.append(caller_call)
            if caller_call.function_name not in reachable_functions:
                reachable_functions.add(caller_call.function_name)
                queue.append(caller_call.function_name)

    return reachable_functions, reachable_edges


def format_cleanup_drop(drop: CleanupDrop) -> str:
    detail_parts: list[str] = []
    if drop.debug_names:
        detail_parts.append("/".join(drop.debug_names))
    if drop.root_local is not None and drop.root_local != drop.place:
        detail_parts.append(f"root {drop.root_local}")
    if drop.type_name:
        detail_parts.append(drop.type_name)
    detail = f" [{' | '.join(detail_parts)}]" if detail_parts else ""
    return f"{drop.block_name}@{drop.line_no}: {drop.text}{detail}"


def format_cleanup_lines(drops: list[CleanupDrop]) -> str:
    return ", ".join(format_cleanup_drop(drop) for drop in drops[:4])


def print_report(
    target_calls: list[CallSite],
    reachable_functions: set[str],
    reachable_edges: list[CallSite],
    quiet: bool,
) -> int:
    unsafe_target_calls = [call for call in target_calls if call.has_drop_cleanup]
    unsafe_edges = [call for call in reachable_edges if call.has_drop_cleanup]

    if not quiet:
        print("Direct out_of_memory callsites:")
        for call in sorted(target_calls, key=lambda item: (item.function_name, item.mir_line)):
            status = "UNSAFE" if call.has_drop_cleanup else "SAFE"
            print(
                f"  {status} {call.function_name} @ MIR line {call.mir_line} "
                f"calls {call.callee_expr}"
            )
            if call.has_drop_cleanup:
                print(f"    cleanup drops: {format_cleanup_lines(call.cleanup_drops)}")

        print()
        print("Reachable caller edges:")
        for call in sorted(reachable_edges, key=lambda item: (item.function_name, item.mir_line)):
            status = "UNSAFE" if call.has_drop_cleanup else "SAFE"
            callees = ", ".join(sorted(call.callee_functions))
            print(
                f"  {status} {call.function_name} @ MIR line {call.mir_line} "
                f"calls {call.callee_expr}"
            )
            if callees:
                print(f"    resolved callees: {callees}")
            if call.has_drop_cleanup:
                print(f"    cleanup drops: {format_cleanup_lines(call.cleanup_drops)}")

        print()
        print(f"Reachable in-crate functions: {len(reachable_functions)}")
        for function_name in sorted(reachable_functions):
            print(f"  {function_name}")

    if unsafe_target_calls or unsafe_edges:
        print()
        print(
            f"FAIL: found {len(unsafe_target_calls)} unsafe direct out_of_memory callsite(s) "
            f"and {len(unsafe_edges)} unsafe caller edge(s).",
            file=sys.stderr,
        )
        return 1

    if not quiet:
        print()
    print(
        "PASS: all discovered in-crate stack edges leading to out_of_memory have no drop cleanup."
    )
    return 0


def main() -> int:
    args = parse_args()
    manifest_path = args.manifest_path.resolve()
    crate_root = manifest_path.parent
    mir_path = args.mir.resolve() if args.mir else emit_mir(args)

    try:
        functions = parse_mir(mir_path)
        overrides = parse_trait_impls(crate_root)
        callers_by_callee = resolve_calls(functions, overrides)
        analyze_cleanup(functions)
        target_calls = find_target_calls(functions, args.target_substring)
        if not target_calls:
            print("FAIL: no direct out_of_memory callsites matched the requested target substrings.")
            return 1

        reachable_functions, reachable_edges = reverse_reachable_functions(
            target_calls, callers_by_callee
        )
        return print_report(
            target_calls,
            reachable_functions,
            reachable_edges,
            args.quiet,
        )
    finally:
        if not args.keep_mir and not args.mir:
            mir_path.unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
