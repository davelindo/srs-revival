"""Check public Git candidates for non-text artifacts; optionally check site links."""

import argparse
from html.parser import HTMLParser
from pathlib import Path
import re
import subprocess
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
TEXT_SUFFIXES = {".md", ".txt", ".py", ".css", ".html", ".yml", ".yaml"}
TEXT_NAMES = {"LICENSE", ".gitignore"}
PORT_SUFFIXES = {".rs", ".toml", ".nix", ".c", ".S", ".ld", ".js"}


def check_candidates():
    names = subprocess.check_output(
        ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        cwd=ROOT,
    ).decode().split("\0")
    checked = 0
    for name in sorted(set(filter(None, names))):
        path = ROOT / name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"non-regular public file: {name}")
        port_source = name.startswith("firmware/port/") and path.suffix in PORT_SUFFIXES
        kit_input = name in {"firmware/port/x7/Cargo.lock", "firmware/zd1-port.patch"}
        if path.suffix not in TEXT_SUFFIXES and path.name not in TEXT_NAMES and not (port_source or kit_input):
            raise ValueError(f"unapproved public artifact type: {name}")
        raw = path.read_bytes()
        if b"\0" in raw:
            raise ValueError(f"binary content in {name}")
        text = raw.decode("utf-8")
        if re.search(r"-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----", text):
            raise ValueError(f"private-key material in {name}")
        if re.search(r"\bgh[pousr]_[A-Za-z0-9]{30,}\b", text):
            raise ValueError(f"GitHub credential-like string in {name}")
        checked += 1
    print(f"Publication check: {checked} public text files; no binary artifacts")


class Links(HTMLParser):
    def __init__(self, text):
        super().__init__()
        self.ids = set()
        self.links = []
        self.feed(text)

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if "id" in attrs:
            self.ids.add(attrs["id"])
        for key in ("href", "src"):
            if attrs.get(key):
                self.links.append(attrs[key])


def check_links(site):
    site = site.resolve()
    pages = {path: Links(path.read_text(encoding="utf-8")) for path in site.glob("*.html")}
    if not pages:
        raise ValueError("no built HTML pages")
    count = 0
    for path, parsed in pages.items():
        for href in parsed.links:
            url = urlsplit(href)
            if url.scheme or url.netloc:
                continue
            target = (path.parent / unquote(url.path)).resolve() if url.path else path
            if not target.is_relative_to(site) or not target.is_file():
                raise ValueError(f"{path.name}: missing/internal-outside link {href}")
            if url.fragment and (target not in pages or unquote(url.fragment) not in pages[target].ids):
                raise ValueError(f"{path.name}: missing fragment {href}")
            count += 1
    print(f"Site check: {len(pages)} pages, {count} valid internal links/assets")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--site", type=Path)
    args = parser.parse_args()
    check_candidates()
    if args.site:
        check_links(args.site)


if __name__ == "__main__":
    main()
