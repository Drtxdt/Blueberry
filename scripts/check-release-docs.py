"""Check published status independently from the development Cargo version."""
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
def check():
    receipts = list((ROOT / "docs/benchmarks").rglob("*-publication-receipt.json"))
    published = []
    for path in receipts:
        record = json.loads(path.read_text(encoding="utf-8-sig"))
        match = re.search(r"/tag/v(\d+\.\d+\.\d+)$", record.get("release_url", ""))
        if match and record.get("published_at"):
            published.append((tuple(map(int, match[1].split("."))), match[1], record["published_at"][:10]))
    if not published:
        raise ValueError("No published release receipt")
    _, version, date = max(published)
    readme = (ROOT / "README.md").read_text(encoding="utf-8-sig")
    if f"当前公开版本为 [v{version}]" not in readme or f"已于 {date} 发布" not in readme:
        raise ValueError("README stable version differs from publication receipt")
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8-sig")
    section = changelog.split(f"## {version}\n", 1)[1].split("\n## ", 1)[0]
    if "尚未公开" in section or f"已于 {date} 公开" not in section:
        raise ValueError("CHANGELOG published status is stale")
    print(f"Published documentation consistent: {version}")
if __name__ == "__main__":
    check()
