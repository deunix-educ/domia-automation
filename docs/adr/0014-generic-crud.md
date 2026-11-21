# 0014 — Generic CRUD component for administration pages

- Status: accepted
- Date: 2026-11-01

## Context
A generated administration site typically provides about 30 CRUD screens. The first phase replaces four (places, navigation, settings, users) and the following phases add cameras, devices, scenes, etc. Writing each screen by hand would multiply interface code, error cases and translations.

## Options
1. **A generic component driven by a field description** (`FieldSpec`), and concrete server functions per resource — chosen.
2. **Everything generic, server side included** (a "save any resource" server function with textual values) — less code, but validation and rights would go through a weakly typed contract, and the target table becomes data sent by the browser.
3. **A hand-written page per resource** — simplest to read, longest to maintain.
4. **A `derive` macro generating page and server functions** — elegant, but costly to write and debug for 4 resources.

## Decision
- `crud.rs`: `CrudTable` (filterable table, add/edit form, in-place deletion confirmation, per-field errors). A page supplies the `FieldSpec`s (name, translation key, type text / text area / checkbox, mandatory, maximum length, table column), the list (`Signal<Vec<Row>>`) and **two `Action`s** (save, delete) that call its server functions. The component knows neither the business type nor the network.
- Server functions stay **concrete and typed** per resource. Each starts with `require_admin` (403 otherwise): page guards only serve display. Expected refusals are **values** (`SaveOutcome::{Saved, Invalid, NotFound}`, `DeleteOutcome::{Deleted, NotFound, InUse}`), not HTTP errors; only failures are errors (logged).
- Validation is authoritative on the server and reports **field by field** (`FieldProblem { field, kind }`) so the interface translates the message under the right field. The domain carries the rule (`LocationDraft::problems`), `validate` stays for the API.
- Labels are **keys** (`tk!("key")`, resolved by `I18n::tr` at display time to follow the language). `tk!` makes the key findable: `tests/i18n.rs` checks that it exists in both languages and that no key is orphaned.
- Displayed content is always escaped by Leptos (test with `<script>` in a name).

## Consequences
- Current field types: text, text area, checkbox, bounded number, drop-down list, password, display-only column and a field that cannot be changed after creation. A `Blocked` result expresses a business refusal (neither an error nor an invalid field). Settings (fixed keys, typed values, secrets) are **not** CRUD: a dedicated page on the same typed server functions.
- A new simple resource = a field description, three server functions, a page of about fifty lines.
- Resources with relations, secret fields or business rules (last administrator) add field types or result variants; the component evolves with real needs.
- No duplicated REST API for these screens: the interface goes through server functions (ADR 0012).
