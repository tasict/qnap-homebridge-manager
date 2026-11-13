#!/usr/bin/env python3
"""
Check the web UI's translations.

    python3 tools/i18n-check.py            # check, exit 1 on a problem
    python3 tools/i18n-check.py --keys     # print every key the UI uses

What it checks (shared/web):
  - every key used in the code is in i18n/zh-TW.json and i18n/ja.json;
  - every key in those two catalogs is used somewhere (no leftovers);
  - a translation has exactly the {placeholders} of its key;
  - the catalogs are flat JSON objects of non-empty strings, sorted by key;
  - no Chinese or Japanese text is left in js/, css/ or index.html outside a
    catalog (comments and the two language names are allowed).
It also reports, without failing, backend keys (i18n/<lang>.backend.json)
that are in one language and not the other, or whose placeholders differ.

Keys are found as the first, literal argument of t(), tn() and N_() (also
called as HB.t, HB.tn, HB.N_), and in index.html's data-t, data-t-aria-label
and data-t-title attributes. A call with a computed key is listed as a note:
such keys must come from a table marked with N_().
"""

import json
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "shared", "web"))
LANGS = ("zh-TW", "ja")
ALLOWED_CJK = ("繁體中文", "日本語")

CALL = re.compile(r"(?<![\w$])(?:HB\.)?(t|tn|N_)\(")
STRING = re.compile(r"""\s*('(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*")""")
ATTR = re.compile(r"""data-t(?:-aria-label|-title)?="([^"]*)\"""")
PLACEHOLDER = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")
CJK = re.compile(r"[　-ヿ㐀-䶿一-鿿豈-﫿＀-￯]")


def unescape(lit):
    body = lit[1:-1]
    out = []
    i = 0
    while i < len(body):
        c = body[i]
        if c == "\\" and i + 1 < len(body):
            n = body[i + 1]
            simple = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", "'": "'", '"': '"', "0": "\0"}
            if n in simple:
                out.append(simple[n])
                i += 2
                continue
            if n == "u" and body[i + 2:i + 3] == "{":
                end = body.index("}", i)
                out.append(chr(int(body[i + 3:end], 16)))
                i = end + 1
                continue
            if n == "u":
                out.append(chr(int(body[i + 2:i + 6], 16)))
                i += 6
                continue
            if n == "x":
                out.append(chr(int(body[i + 2:i + 4], 16)))
                i += 4
                continue
            out.append(n)
            i += 2
            continue
        out.append(c)
        i += 1
    return "".join(out)


def strip_comments(src):
    """Blank out /* */ and // comments, leaving strings alone (good enough for this code base)."""
    out = []
    i = 0
    n = len(src)
    quote = None
    while i < n:
        c = src[i]
        if quote:
            out.append(c)
            if c == "\\" and i + 1 < n:
                out.append(src[i + 1])
                i += 2
                continue
            if c == quote:
                quote = None
            i += 1
            continue
        if c in "'\"`":
            quote = c
            out.append(c)
            i += 1
            continue
        if src.startswith("/*", i):
            end = src.find("*/", i + 2)
            end = n if end < 0 else end + 2
            out.append(re.sub(r"[^\n]", " ", src[i:end]))
            i = end
            continue
        if src.startswith("//", i) and (i == 0 or src[i - 1] in " \t\n;{}(),"):
            end = src.find("\n", i)
            end = n if end < 0 else end
            out.append(" " * (end - i))
            i = end
            continue
        out.append(c)
        i += 1
    return "".join(out)


def scan_code():
    used = {}
    dynamic = []
    leftovers = []
    js_dir = os.path.join(ROOT, "js")
    files = sorted(os.path.join(js_dir, f) for f in os.listdir(js_dir) if f.endswith(".js"))
    for path in files:
        rel = os.path.relpath(path, ROOT)
        with open(path, encoding="utf-8") as fh:
            src = fh.read()
        code = strip_comments(src)
        for m in CALL.finditer(code):
            line = code.count("\n", 0, m.start()) + 1
            before = code[max(0, m.start() - 9):m.start()]
            if before.endswith("function "):
                continue
            if before.endswith(".") and not before.endswith("..."):
                continue  # obj.t(...) is some other method
            s = STRING.match(code, m.end())
            if s:
                key = unescape(s.group(1))
                used.setdefault(key, []).append("%s:%d" % (rel, line))
            elif not rel.endswith("i18n.js"):
                dynamic.append("%s:%d" % (rel, line))
        for no, text in enumerate(code.split("\n"), 1):
            stripped = text
            for word in ALLOWED_CJK:
                stripped = stripped.replace(word, "")
            if CJK.search(stripped):
                leftovers.append("%s:%d: %s" % (rel, no, text.strip()[:100]))
    index = os.path.join(ROOT, "index.html")
    with open(index, encoding="utf-8") as fh:
        html = fh.read()
    for m in ATTR.finditer(html):
        used.setdefault(m.group(1).replace("&amp;", "&").replace("&quot;", '"'), []).append("index.html")
    for no, text in enumerate(re.sub(r"<!--.*?-->", "", html, flags=re.S).split("\n"), 1):
        if CJK.search(text):
            leftovers.append("index.html:%d: %s" % (no, text.strip()[:100]))
    css = os.path.join(ROOT, "css", "app.css")
    with open(css, encoding="utf-8") as fh:
        css_code = re.sub(r"/\*.*?\*/", lambda mm: re.sub(r"[^\n]", " ", mm.group(0)), fh.read(), flags=re.S)
    for no, text in enumerate(css_code.split("\n"), 1):
        if CJK.search(text):
            leftovers.append("css/app.css:%d: %s" % (no, text.strip()[:100]))
    return used, dynamic, leftovers


def load(path, problems, label):
    if not os.path.exists(path):
        return None
    try:
        with open(path, encoding="utf-8") as fh:
            raw = fh.read()
        data = json.loads(raw)
    except ValueError as exc:
        problems.append("%s: not valid JSON (%s)" % (label, exc))
        return {}
    if not isinstance(data, dict):
        problems.append("%s: not a JSON object" % label)
        return {}
    for k, v in data.items():
        if not isinstance(v, str) or not v.strip():
            problems.append("%s: %r has an empty or non-text translation" % (label, k))
    if list(data.keys()) != sorted(data.keys()):
        problems.append("%s: keys are not sorted" % label)
    return data


def placeholders(text):
    return sorted(set(PLACEHOLDER.findall(text)))


def main():
    used, dynamic, leftovers = scan_code()
    if "--keys" in sys.argv:
        for k in sorted(used):
            print(k)
        return 0
    problems = []
    notes = []
    for lang in LANGS:
        label = "i18n/%s.json" % lang
        cat = load(os.path.join(ROOT, "i18n", lang + ".json"), problems, label)
        if cat is None:
            problems.append("%s is missing" % label)
            continue
        for key in sorted(used):
            if key not in cat:
                problems.append("%s: missing %r (used at %s)" % (label, key, ", ".join(used[key][:3])))
        for key in sorted(cat):
            if key not in used:
                problems.append("%s: %r is not used anywhere" % (label, key))
            elif placeholders(key) != placeholders(cat[key]):
                problems.append("%s: placeholders of %r differ: %s vs %s" % (label, key, placeholders(key), placeholders(cat[key])))
    for item in leftovers:
        problems.append("text outside the catalogs: " + item)

    backend = {}
    for lang in LANGS:
        label = "i18n/%s.backend.json" % lang
        data = load(os.path.join(ROOT, "i18n", lang + ".backend.json"), notes, label)
        backend[lang] = data
        if data is None:
            notes.append("%s is not there yet" % label)
            continue
        for key, value in data.items():
            if isinstance(value, str) and placeholders(key) != placeholders(value):
                notes.append("%s: placeholders of %r differ: %s vs %s" % (label, key, placeholders(key), placeholders(value)))
    for lang in LANGS:
        ui_path = os.path.join(ROOT, "i18n", lang + ".json")
        data = backend.get(lang)
        if data and os.path.exists(ui_path):
            with open(ui_path, encoding="utf-8") as fh:
                ui = json.load(fh)
            for key in sorted(set(ui) & set(data)):
                if ui[key] != data[key]:
                    notes.append("i18n/%s: %r is in both catalogs with different wording; the UI's is shown (%r, not %r)"
                                 % (lang, key, ui[key], data[key]))
    a, b = backend.get("zh-TW"), backend.get("ja")
    if a is not None and b is not None:
        for key in sorted(set(a) - set(b)):
            notes.append("backend key only in zh-TW: %r" % key)
        for key in sorted(set(b) - set(a)):
            notes.append("backend key only in ja: %r" % key)

    for d in dynamic:
        notes.append("computed key (must come from an N_() table): " + d)
    for n in notes:
        print("note: " + n)
    for p in problems:
        print("problem: " + p)
    print("%d keys used by the UI, %d problems, %d notes" % (len(used), len(problems), len(notes)))
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
