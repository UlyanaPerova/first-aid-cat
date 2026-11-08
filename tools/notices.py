#!/usr/bin/env python3
"""Таблица «Rust crates» в release/THIRD_PARTY_NOTICES.md — из Cargo.lock (cargo metadata), а не руками.

  python3 tools/notices.py          — переписать таблицу
  python3 tools/notices.py --check  — код 1, если таблица не совпадает с Cargo.lock (для CI)

Берутся все пакеты, до которых можно дойти от aptechka по обычным и сборочным зависимостям (на любой
системе: Linux, Windows, macOS); только тестовые — нет. Свои копии по пути (vendor/softbuffer) — в разделе
«Vendored code», не здесь.
"""
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
NOTICES = next(p for p in (os.path.join(ROOT, "release", "THIRD_PARTY_NOTICES.md"), os.path.join(ROOT, "THIRD_PARTY_NOTICES.md")) if os.path.exists(p))
HEAD = "## Rust crates"


def table():
    meta = json.loads(subprocess.run(["cargo", "metadata", "--format-version", "1", "--locked"], cwd=os.path.join(ROOT, "engine"),
                                     check=True, capture_output=True, text=True).stdout)
    pk = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    root = meta["resolve"]["root"]
    seen, todo = set(), [root]
    while todo:
        i = todo.pop()
        if i in seen:
            continue
        seen.add(i)
        for d in nodes[i]["deps"]:
            if any(k.get("kind") != "dev" for k in d["dep_kinds"]):
                todo.append(d["pkg"])
    rows = []
    for i in seen:
        p = pk[i]
        if i == root or p.get("source") is None:  # сама программа и копии по пути
            continue
        lic = (p.get("license") or ("see " + p["license_file"] if p.get("license_file") else "unknown")).replace("/", " OR ")
        src = p.get("repository") or p.get("homepage") or f"https://crates.io/crates/{p['name']}"
        rows.append((p["name"], p["version"], lic, src))
    rows.sort(key=lambda r: (r[0].lower(), r[1]))
    out = ["| Crate | Version | License | Source |", "|---|---|---|---|"]
    out += [f"| {n} | {v} | {l} | {s} |" for n, v, l, s in rows]
    return "\n".join(out) + "\n"


def main():
    text = open(NOTICES, encoding="utf-8").read()
    i = text.index(HEAD)
    j = text.index("| Crate |", i)
    new = text[:j] + table()
    if "--check" in sys.argv:
        if new != text:
            print("THIRD_PARTY_NOTICES.md не совпадает с Cargo.lock: python3 tools/notices.py")
            sys.exit(1)
        print("THIRD_PARTY_NOTICES.md совпадает с Cargo.lock")
        return
    open(NOTICES, "w", encoding="utf-8").write(new)
    print(f"{NOTICES}: {new.count(chr(10)) - text[:j].count(chr(10)) - 2} пакетов")


if __name__ == "__main__":
    main()
