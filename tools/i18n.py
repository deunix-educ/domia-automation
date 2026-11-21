#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Langues de l'interface (Fluent, `crates/dominet-web/locales/<code>/main.ftl`).

  tools/i18n.py new CODE [--from fr|en]   cree une langue a partir de l'anglais (ou du francais)
  tools/i18n.py status [--list]           messages a traduire et ecarts de cles / de variables, par langue

Une langue ajoutee apparait seule dans le selecteur apres reconstruction (`make front`) : les traductions sont compilees dans le
binaire (leptos-fluent) et le nom affiche vient du code. Le code doit etre connu de leptos-fluent (`es`, `de`, `it`, `pt-BR`...) :
sinon la compilation s'arrete en le disant. Le francais et l'anglais restent les langues de reference (documentation, E2E).
"""

import argparse
import re
import sys
from pathlib import Path

LOCALES = Path(__file__).resolve().parent.parent / "crates/dominet-web/locales"
REFERENCE = "en"
CODE = re.compile(r"^[a-z]{2,3}(-[A-Z]{2})?$")
KEY = re.compile(r"^([A-Za-z][A-Za-z0-9_-]*)\s*=(.*)$")
VAR = re.compile(r"\$([A-Za-z][\w-]*)")


def parse(path: Path) -> dict[str, str]:
    """Messages d'un fichier `.ftl` : cle -> valeur (lignes de continuation et attributs inclus)."""
    messages: dict[str, str] = {}
    key = None
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("#"):
            continue
        m = KEY.match(line)
        if m:
            key = m.group(1)
            messages[key] = m.group(2).strip()
        elif key and line[:1] in (" ", "\t"):
            messages[key] += "\n" + line.strip()
        elif line.strip():
            key = None
    return messages


def languages() -> list[str]:
    return sorted(p.name for p in LOCALES.iterdir() if (p / "main.ftl").is_file())


def cmd_new(args: argparse.Namespace) -> int:
    code, source = args.code, args.source
    if not CODE.match(code):
        print(f"code de langue invalide : « {code} » (attendu : fr, es, pt-BR...)", file=sys.stderr)
        return 2
    target = LOCALES / code
    if target.exists():
        print(f"la langue {code} existe deja : {target}", file=sys.stderr)
        return 2
    origin = LOCALES / source / "main.ftl"
    if not origin.is_file():
        print(f"langue de depart introuvable : {origin}", file=sys.stderr)
        return 2
    text = origin.read_text(encoding="utf-8")
    first, _, rest = text.partition("\n")
    note = (
        f"# Traduction « {code} » creee depuis « {source} » : traduire chaque valeur, sans toucher aux cles ni aux variables ($nom).\n"
        "# Suivi : `make i18n-status`. Reconstruire l'interface (`make front`) pour la voir dans le selecteur de langue.\n"
    )
    target.mkdir(parents=True)
    (target / "main.ftl").write_text(first + "\n" + note + rest, encoding="utf-8")
    print(f"cree : {target / 'main.ftl'} ({len(parse(target / 'main.ftl'))} messages, tous a traduire)")
    print("suite : traduire le fichier, `make i18n-status`, puis `make front` et `make ci`.")
    return 0


def cmd_status(args: argparse.Namespace) -> int:
    ref = parse(LOCALES / REFERENCE / "main.ftl")
    fr = parse(LOCALES / "fr" / "main.ftl")
    failed = False
    for code in languages():
        if code == REFERENCE:
            print(f"{code}: reference ({len(ref)} messages)")
            continue
        msgs = parse(LOCALES / code / "main.ftl")
        missing = sorted(set(ref) - set(msgs))
        extra = sorted(set(msgs) - set(ref))
        bad_vars = sorted(k for k in msgs if k in ref and set(VAR.findall(msgs[k])) != set(VAR.findall(ref[k])))
        # Identique a l'anglais alors que le francais differe : tres probablement pas encore traduit. Les textes que
        # le francais partage avec l'anglais (« OK », « Grafana »...) sont neutres et ne comptent pas.
        todo = sorted(k for k in msgs if k in ref and msgs[k] == ref[k] and fr.get(k) != ref[k])
        print(f"{code}: {len(msgs)} messages, a traduire (identiques a l'anglais) : {len(todo)}"
              f", cles manquantes : {len(missing)}, en trop : {len(extra)}, variables differentes : {len(bad_vars)}")
        if missing or extra or bad_vars:
            failed = True
            for label, keys in (("manquantes", missing), ("en trop", extra), ("variables differentes", bad_vars)):
                for k in keys[:20]:
                    print(f"    {label} : {k}")
        if args.list:
            for k in todo:
                print(f"    a traduire : {k}")
    return 1 if failed else 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="cmd", required=True)
    new = sub.add_parser("new", help="cree une langue")
    new.add_argument("code")
    new.add_argument("--from", dest="source", default=REFERENCE, choices=["en", "fr"])
    new.set_defaults(run=cmd_new)
    status = sub.add_parser("status", help="etat des langues")
    status.add_argument("--list", action="store_true", help="liste les cles a traduire")
    status.set_defaults(run=cmd_status)
    args = parser.parse_args()
    return args.run(args)


if __name__ == "__main__":
    sys.exit(main())
