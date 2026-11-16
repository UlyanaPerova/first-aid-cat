#!/usr/bin/env python3
"""Покрытие перевода: русские строки в engine/src против таблиц engine/i18n/en/*.txt.

  python3 tools/i18n.py check              — сколько строк без перевода (по модулям); код 1, если есть
  python3 tools/i18n.py missing MODULE     — непереведённые строки модуля MODULE.rs в формате таблицы
                                             (ru: …  /  en: пусто) — дописать в engine/i18n/en/MODULE.txt
  python3 tools/i18n.py missing custom     — то же для своих проверок (custom/mod.rs → custom/en.txt)
  python3 tools/i18n.py unused             — строки таблиц, которых в коде уже нет (убрать вручную)
Строки, которые никогда не показываются (имена слоёв картинки, служебные ключи, разбор чужого вывода), —
в engine/i18n/ignore/MODULE.txt (по одной, в том же виде, что после «ru: »; # — комментарий).
"""
import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.realpath(__file__)))
SRC = os.path.join(ROOT, "engine", "src")
TABLES = os.path.join(ROOT, "engine", "i18n", "en")
IGNORE = os.path.join(ROOT, "engine", "i18n", "ignore")
CYR = re.compile(r"[А-Яа-яЁё]")
FMT_MACROS = re.compile(r"(?:format|println|eprintln|print|eprint|panic|format_args|write|writeln)!\s*\(\s*(?:[A-Za-z_][\w.]*\s*,\s*)?$")


def rust_strings(text):
    """(строка, это_формат) для всех строковых литералов вне комментариев и тестов."""
    out = []
    i, n = 0, len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            j = text.find("*/", i + 2)
            i = n if j < 0 else j + 2
            continue
        if c == "'":  # символ 'x' или '\n' или время жизни 'a
            m = re.match(r"'(\\.|\\u\{[0-9a-fA-F]+\}|[^\\'])'", text[i:])
            i += len(m.group(0)) if m else 1
            continue
        m = re.match(r'(b?)r(#*)"', text[i:])
        if m and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = m.group(2)
            start = i + len(m.group(0))
            end = text.find('"' + hashes, start)
            out.append((text[start:end], i, False))
            i = end + 1 + len(hashes)
            continue
        if c == '"':
            j = i + 1
            buf = []
            while j < n and text[j] != '"':
                if text[j] == "\\":
                    e = text[j + 1]
                    if e == "n":
                        buf.append("\n")
                    elif e == "t":
                        buf.append("\t")
                    elif e == "r":
                        buf.append("\r")
                    elif e == "0":
                        buf.append("\0")
                    elif e == "u":
                        k = text.index("}", j)
                        buf.append(chr(int(text[j + 3:k], 16)))
                        j = k + 1
                        continue
                    elif e == "\n":  # перенос строки внутри литерала: пробелы в начале следующей пропускаются
                        j += 2
                        while j < n and text[j] in " \t\n":
                            j += 1
                        continue
                    else:
                        buf.append(e)
                    j += 2
                    continue
                buf.append(text[j])
                j += 1
            out.append(("".join(buf), i, None))
            i = j + 1
            continue
        i += 1
    res = []
    for s, pos, raw in out:
        before = text[max(0, pos - 80):pos]
        is_fmt = bool(FMT_MACROS.search(before))
        res.append((s, pos, is_fmt))
    return res


def test_ranges(text):
    """Области #[cfg(test)] mod … { … } — их строки не показываются пользователю."""
    rng = []
    for m in re.finditer(r"#\[cfg\(test\)\]\s*(?:pub\s+)?mod\s+\w+\s*\{", text):
        depth, j = 1, m.end()
        while j < len(text) and depth:
            depth += {"{": 1, "}": -1}.get(text[j], 0)
            j += 1
        rng.append((m.start(), j))
    return rng


def normalize(s, is_fmt):
    """Как строка выглядит в таблице: в format! любая {…} → {}, у простой строки скобки удваиваются."""
    if is_fmt:
        return re.sub(r"\{(?!\{)[^{}]*\}", "{}", s.replace("{{", "\0L").replace("}}", "\0R")).replace("\0L", "{{").replace("\0R", "}}")
    return s.replace("{", "{{").replace("}", "}}")


def esc(s):
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t")


def unesc(s):
    out, it = [], iter(s)
    for c in it:
        if c == "\\":
            e = next(it, "\\")
            out.append({"n": "\n", "t": "\t", "s": " "}.get(e, e))
        else:
            out.append(c)
    return "".join(out)


CUSTOM = os.path.join(ROOT, "custom")  # свои проверки (CUSTOM.md): перевод — в custom/en.txt


def load_tables():
    done, pending = set(), set()
    if not os.path.isdir(TABLES):
        return done, pending
    files = [os.path.join(TABLES, f) for f in os.listdir(TABLES) if f.endswith(".txt")]
    if os.path.isfile(os.path.join(CUSTOM, "en.txt")):
        files.append(os.path.join(CUSTOM, "en.txt"))
    for f in files:
        ru = None
        for line in open(f, encoding="utf-8"):
            line = line.rstrip("\n").rstrip("\r")
            if line.startswith("ru: "):
                ru = unesc(line[4:])
            elif line.startswith("en: ") and ru is not None:
                (done if line[4:].strip() else pending).add(ru)
                if "|" in ru:  # формы числа: каждая по отдельности тоже считается переведённой
                    for p in ru.split("|"):
                        (done if line[4:].strip() else pending).add(p)
                ru = None
    return done, pending


def load_ignore():
    out = set()
    if os.path.isdir(IGNORE):
        for f in os.listdir(IGNORE):
            if f.endswith(".txt"):
                out |= {unesc(l.rstrip("\n")) for l in open(os.path.join(IGNORE, f), encoding="utf-8") if l.strip() and not l.startswith("#")}
    return out


def module_strings(path):
    text = open(path, encoding="utf-8").read()
    tests = test_ranges(text)
    seen, out = set(), []
    for s, pos, is_fmt in rust_strings(text):
        if not CYR.search(s) or any(a <= pos < b for a, b in tests):
            continue
        key = normalize(s, is_fmt)
        if key not in seen:
            seen.add(key)
            out.append((key, text.count("\n", 0, pos) + 1))
    return out


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "check"
    done, pending = load_tables()
    ign = load_ignore()
    mods = sorted(f[:-3] for f in os.listdir(SRC) if f.endswith(".rs"))
    if cmd == "missing":
        mod = sys.argv[2]
        path = os.path.join(CUSTOM, "mod.rs") if mod == "custom" else os.path.join(SRC, mod + ".rs")
        for key, line in module_strings(path):
            if key not in done and key not in ign:
                print(f"# {mod}.rs:{line}\nru: {esc(key)}\nen: \n")
        return 0
    if cmd == "unused":
        used = {k for m in mods for k, _ in module_strings(os.path.join(SRC, m + ".rs"))}
        # строки, собранные в коде из кусков («Мур! Беда:» + имя + «—» + значение), целиком в коде не встречаются:
        # такая строка жива, если каждый её русский кусок между {} есть в исходниках
        src = "".join(open(os.path.join(SRC, m + ".rs"), encoding="utf-8").read() for m in mods)
        assets = os.path.join(SRC, "..", "assets")  # фразы кота — из текстового файла, а не из кода
        src += "".join(open(os.path.join(assets, f), encoding="utf-8").read() for f in os.listdir(assets) if f.endswith(".txt"))

        def alive(ru):
            parts = [p.strip(" .,:;!?…()[]«»—-\n") for p in re.split(r"\{\}|\|", ru)]
            parts = [p for p in parts if CYR.search(p)]
            return not parts or all(esc(p) in src or p in src for p in parts)  # без русских слов — не судим

        n = 0
        for f in sorted(os.listdir(TABLES)):
            if not f.endswith(".txt"):
                continue
            for line in open(os.path.join(TABLES, f), encoding="utf-8"):
                if line.startswith("ru: "):
                    ru = unesc(line[4:].rstrip("\n").rstrip("\r"))
                    if ru not in used and not any(p in used for p in ru.split("|")) and not alive(ru):
                        print(f"{f}: {line[4:].rstrip()}")
                        n += 1
        print(f"{n:5}  строк в таблицах, которых в коде уже нет")
        return 0
    total = 0
    for m in mods:
        miss = [k for k, _ in module_strings(os.path.join(SRC, m + ".rs")) if k not in done and k not in ign]
        if miss:
            print(f"{len(miss):5}  {m}.rs")
            total += len(miss)
    if os.path.isfile(os.path.join(CUSTOM, "mod.rs")):
        miss = [k for k, _ in module_strings(os.path.join(CUSTOM, "mod.rs")) if k not in done and k not in ign]
        if miss:
            print(f"{len(miss):5}  custom/mod.rs  (перевод — в custom/en.txt: python3 tools/i18n.py missing custom)")
            total += len(miss)
    print(f"{total:5}  всего без перевода")
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
