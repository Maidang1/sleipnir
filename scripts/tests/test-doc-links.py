import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "doc_links", Path(__file__).resolve().parents[1] / "check-doc-links.py"
)
doc_links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(doc_links)


class DocLinkTests(unittest.TestCase):
    def test_commonmark_links_images_references_and_html(self):
        text = """
[inline](docs/a.md#heading)
![image](a%20b.png)
[reference][guide]

[guide]: docs/b.md "Guide"

<img src="icon.png"><a href="README.md">Home</a>
`[not a link](missing.md)`

```md
[also not a link](missing.md)
```
"""
        self.assertEqual(
            list(doc_links.link_targets(text)),
            ["docs/a.md#heading", "a%20b.png", "docs/b.md", "icon.png", "README.md"],
        )

    def test_missing_local_targets_and_deleted_files_fail(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            docs = root / "docs"
            docs.mkdir()
            target = root / "a b.md"
            target.touch()
            source = docs / "index.md"
            source.write_text(
                "[ok](../a%20b.md#heading)\n[root](/a%20b.md)\n"
                "[fragment](#heading)\n[web](https://example.com/x)\n"
                "[mail](mailto:user@example.com)\n[missing](gone.md)\n",
                encoding="utf-8",
            )
            self.assertEqual(list(doc_links.missing_targets(source, root)), ["gone.md"])
            target.unlink()
            self.assertEqual(
                list(doc_links.missing_targets(source, root)),
                ["../a%20b.md#heading", "/a%20b.md", "gone.md"],
            )


if __name__ == "__main__":
    unittest.main()
