#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Extrait les icones de `lucide.min.js` (ISC) vers `crates/dominet-server/data/lucide.tsv`.

Usage : python3 tools/gen-lucide.py [chemin/lucide.min.js] [sortie.tsv]

Format de sortie : une ligne par icone, `nom1 nom2 ...<TAB>contenu SVG interieur`, ou les noms sont les alias en
kebab-case (`settings-2`, `wifi-sync`...). Le contenu ne contient que des balises de dessin d'une liste blanche et des
attributs de geometrie : aucun script, aucun style, aucun lien. L'enveloppe `<svg ...>` est ajoutee a l'execution.
Aucune dependance externe ; le script n'est utile qu'a la mise a jour de Lucide.
"""
import json
import re
import sys
from html import escape

SRC = sys.argv[1] if len(sys.argv) > 1 else "/home/elite/opt/DomiNet/dominet/static/core/js/lucide.min.js"
OUT = sys.argv[2] if len(sys.argv) > 2 else "crates/dominet-server/data/lucide.tsv"
TAGS = {"path", "line", "polyline", "polygon", "circle", "rect", "ellipse"}
ATTR_OK = re.compile(r"^[a-z][a-z0-9-]*$")

src = open(SRC, encoding="utf-8").read()
version = (re.search(r"@license lucide v([0-9.]+)", src) or [None, "inconnue"])[1]


def span(text, start, open_ch="[", close_ch="]"):
    """Fin (exclue) du groupe qui commence en `start`, en ignorant le contenu des chaines."""
    depth, in_str, esc = 0, False, False
    for k in range(start, len(text)):
        c = text[k]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            continue
        if c == '"':
            in_str = True
        elif c == open_ch:
            depth += 1
        elif c == close_ch:
            depth -= 1
            if depth == 0:
                return k + 1
    raise ValueError("groupe non ferme")


def js_to_json(text):
    """Litteral JavaScript minifie (cles non citees, `.5`) -> JSON, sans toucher au contenu des chaines."""
    out, i, in_str, esc = [], 0, False, False
    while i < len(text):
        c = text[i]
        if in_str:
            out.append(c)
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            i += 1
            continue
        if c == '"':
            in_str = True
            out.append(c)
            i += 1
            continue
        m = re.match(r"([A-Za-z_][A-Za-z0-9_-]*)(?=:)", text[i:]) if (out and out[-1] in "{,") else None
        if m:
            out.append(f'"{m.group(1)}"')
            i += len(m.group(1))
            continue
        if c == "." and text[i + 1 : i + 2].isdigit() and not (out and out[-1].isdigit()):
            out.append("0")
        out.append(c)
        i += 1
    return "".join(out)


def to_markup(nodes):
    parts = []
    for node in nodes:
        tag, attrs = node[0], node[1]
        if tag not in TAGS:
            raise ValueError(f"balise non autorisee : {tag}")
        for k in attrs:
            if not ATTR_OK.match(k) or k.startswith("on") or k in {"href", "style", "src"}:
                raise ValueError(f"attribut non autorise : {k}")
        attr_txt = "".join(f' {k}="{escape(str(v), quote=True)}"' for k, v in attrs.items())
        parts.append(f"<{tag}{attr_txt}/>")
    return "".join(parts)


# 1. Definitions: NAME=[["path",{...}],...]
defs = {}
for m in re.finditer(r'(?:[,;]|\bconst |\blet |\bvar )([A-Za-z_$][\w$]*)=\[\["', src):
    start = m.end() - 3  # on the inner "[" of the outer group
    start = src.index("[[", m.start())
    end = span(src, start)
    defs[m.group(1)] = json.loads(js_to_json(src[start:end]))

# 2. Exports: a.PascalName=variable
exports = {}
for m in re.finditer(r"\ba\.([A-Za-z0-9_$]+)=([A-Za-z_$][\w$]*)(?=[,;])", src):
    if m.group(2) in defs:
        exports[m.group(1)] = m.group(2)


def kebab(pascal):
    s = re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", pascal)
    s = re.sub(r"([A-Za-z])(\d)", r"\1-\2", s)
    s = re.sub(r"(\d)([A-Za-z])", r"\1-\2", s) if False else s
    return s.lower()


groups = {}
for pascal, var in exports.items():
    groups.setdefault(var, set()).add(kebab(pascal))
rows = []
for var, names in groups.items():
    markup = to_markup(defs[var])
    rows.append((sorted(names), markup))
rows.sort(key=lambda r: r[0][0])
with open(OUT, "w", encoding="utf-8") as f:
    f.write(f"# lucide v{version} (licence ISC) : genere par tools/gen-lucide.py, ne pas modifier a la main\n")
    for names, markup in rows:
        f.write(" ".join(names) + "\t" + markup + "\n")
print(f"lucide v{version} : {len(rows)} icones distinctes, {sum(len(r[0]) for r in rows)} noms (alias compris) -> {OUT}")
