# SPDX-License-Identifier: AGPL-3.0-or-later
"""Hook MkDocs : les pages venues de la racine du depot (DEPLOY.md) pointent vers `docs/...` ; dans le site, `docs/` est la racine."""


def on_page_markdown(markdown, **_kwargs):
    return markdown.replace("](docs/", "](")
