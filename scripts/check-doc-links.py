#!/usr/bin/env python3
"""Check local Markdown/HTML link targets, without requesting external URLs."""

import argparse
from html.parser import HTMLParser
from pathlib import Path
import subprocess
from urllib.parse import unquote, urlsplit

from markdown_it import MarkdownIt


class HtmlLinks(HTMLParser):
    def __init__(self):
        super().__init__()
        self.targets = []

    def handle_starttag(self, tag, attrs):
        for key, value in attrs:
            if key in ("href", "src") and value:
                self.targets.append(value)


def link_targets(text):
    """Use CommonMark tokens so code examples are not treated as links."""
    def walk(tokens):
        for token in tokens:
            if token.type == "link_open":
                yield token.attrGet("href")
            elif token.type == "image":
                yield token.attrGet("src")
            elif token.type in ("html_inline", "html_block"):
                parser = HtmlLinks()
                parser.feed(token.content)
                yield from parser.targets
            if token.children:
                yield from walk(token.children)

    yield from walk(MarkdownIt("commonmark").parse(text))


def missing_targets(source, root):
    for target in link_targets(source.read_text(encoding="utf-8")):
        url = urlsplit(target)
        if url.scheme or url.netloc or not url.path:
            continue
        path = unquote(url.path)
        resolved = root / path.lstrip("/") if path.startswith("/") else source.parent / path
        if not resolved.exists():
            yield target


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[1])
    args = parser.parse_args()
    root = args.root.resolve()
    files = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=root,
    ).decode().split("\0")
    errors = []
    checked = 0
    for name in sorted(set(files)):
        source = root / name
        if source.suffix.lower() != ".md" or not source.is_file():
            continue
        checked += 1
        errors.extend(f"{name}: missing target {target}" for target in missing_targets(source, root))
    for error in errors:
        print(error)
    print(f"Checked {checked} Markdown files; {len(errors)} missing local targets.")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
