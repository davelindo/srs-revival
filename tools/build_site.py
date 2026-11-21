"""Build the static technical reference from docs/ with Python-Markdown."""

import html
from pathlib import Path
import re
import shutil
from string import Template

import markdown

ROOT = Path(__file__).resolve().parents[1]
PAGES = (
    ("index", "Overview"),
    ("roadmap", "Custom firmware"),
    ("sharing", "Downloads and licensing"),
    ("hardware", "Device architecture"),
    ("spi", "SPI reading"),
    ("telnet", "Enable telnet"),
    ("update-format", "Firmware format"),
    ("emulation", "Local emulation"),
    ("preservation", "Backup and recovery"),
    ("sources", "Sources"),
)


def main():
    output = ROOT / "_site"
    if output.exists():
        shutil.rmtree(output)
    output.mkdir(exist_ok=True)
    expected = {slug for slug, _ in PAGES}
    actual = {path.stem for path in (ROOT / "docs").glob("*.md")}
    if actual != expected:
        raise ValueError(f"navigation and docs disagree: {actual ^ expected}")
    template = Template((ROOT / "site/template.html").read_text(encoding="utf-8"))
    for slug, label in PAGES:
        source = (ROOT / f"docs/{slug}.md").read_text(encoding="utf-8")
        title = source.splitlines()[0].removeprefix("# ")
        renderer = markdown.Markdown(extensions=["tables", "fenced_code", "toc"],
                                      extension_configs={"toc": {"toc_depth": "2-3"}})
        content = renderer.convert(source)
        content = re.sub(r'href="([^"#:]+)\.md(#[^"]*)?"',
                         lambda m: f'href="{m[1]}.html{m[2] or ""}"', content)
        navigation = "\n".join(
            f'<a href="{item}.html"' + (' aria-current="page"' if item == slug else '')
            + f'>{html.escape(name)}</a>' for item, name in PAGES
        )
        rendered = template.substitute(title=html.escape(title), label=html.escape(label),
                                       navigation=navigation, content=content, toc=renderer.toc,
                                       source=f"https://github.com/davelindo/srs-revival/blob/main/docs/{slug}.md")
        (output / f"{slug}.html").write_text(rendered, encoding="utf-8")
    shutil.copyfile(ROOT / "site/style.css", output / "style.css")
    owner_guide = ROOT / "docs/srs-x7.txt"
    shutil.copyfile(owner_guide, output / owner_guide.name)
    (output / ".nojekyll").touch()
    print(f"Built {len(PAGES)} pages in {output}")


if __name__ == "__main__":
    main()
