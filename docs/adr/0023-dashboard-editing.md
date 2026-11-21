# 0023 — Dashboard editing: administrators only, pointer drag, conditional-field form

- Status: accepted
- Date: 2026-11-02

## Context
Dashboards become editable: move a widget, create and correct dashboards and widgets. The plan said "owner or administrator" for moving, whereas the earlier decision and the reference implementation reserve layout to administrators.

## Decision
- **Editing reserved to administrators** (moving, creating, modifying and deleting dashboards and widgets), checked by each server function (403 otherwise, tested for all eight). An ordinary account sees and opens its dashboards but does not modify them; an administrator fills those of all accounts from the administration. Deliberate gap with the plan ("owner or administrator"): letting any owner edit their dashboard would require a second set of rights (fields, widget types, Grafana) that the reference never had.
- **Moving through pointer events** (`drag.rs`) with pointer capture: the same code for mouse, finger and stylus (HTML5 drag and drop does not work with a finger). The widget follows the pointer through a `transform` set by the **CSSOM** (allowed by the CSP, unlike the `style` attribute), a dotted frame shows the target cell, and on release the `move_widget` server function decides: outside the grid or overlapping → refusal, the widget returns to its place and a message explains why. **The server has the last word** (the database exclusion constraint, ADR 0018). Active only with the full 12-column grid (≥ 1201 px): below that, widgets are placed in sequence and their on-screen position is no longer the grid's. No keyboard moving; the administration form (column, row) replaces it.
- **Administration**: `/admin/dashboards` (dashboards of all accounts; the owner designated by user name, immutable afterwards) and `/admin/widgets?d=<id>` (a dashboard's widgets). The generic CRUD component (ADR 0014) gains `field_visible`: a field is only displayed if a page function judges it useful given the current values (device for `device` and `grafana`, scene, camera, content for `text`, JSON configuration for `grafana` and `camera`). Visibility goes through a `Memo` per field: typing in one field does not recreate the others.
- **Validation**: input travels as text (`WidgetInput`) and the server converts it, field by field (`parse_widget`), before the domain rules. An overlap is reported on the `col` field (`ProblemKind::Overlap`). A `text` widget's HTML is **sanitised at save time** (ADR 0022), in addition to display time.

## Pitfalls met (not to repeat)
- **Leptos local storage in a server-rendered component**: a `StoredValue::new_local` (hence `SendWrapper`) created during rendering is released by another thread → `panic` and a frozen page. The drag handler (`drag::Handle`) is ordinary on the server and local only in the browser.
- **A resource whose source depends on another resource** (list of widgets depending on the selected dashboard, itself coming from the dashboard list): at hydration, loads chain out of order and the table stayed stale after a save (100 % failure of end-to-end attempts at 1 in 12 depending on the variant). A single server call (`widgets_page`) now returns the selected dashboard and its widgets.
- **Braces in a Fluent file** (`{"panel_url": …}`): they are expressions; the parse error makes translation loading panic at the first render.
- In end-to-end tests, wait for hydration to finish (the "Real time: connected" indicator) before the first click: before it, a button is present and enabled but has no effect.

## Consequences
- An ordinary account cannot populate its own dashboard: an administrator must do it (or a per-account editing right may be added later).
- A dashboard cannot change owner (delete and recreate).
- `camera` and `scene` widgets save with their reference (UUID, number) from the start.
