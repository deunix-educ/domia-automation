# Scenes

Pages, activation and administration of scenes (R4 lot 2). Decisions: [ADR 0031](adr/0031-automation-schema.md) (schema and domain), R4 plan (D12: permissions).

A scene is a set of commands applied together to several devices ("Evening closing": six shutters and lamps). It has a (unique) name, an icon (emoji or name), a description, a location, an active / inactive state and an **ordered** list of commands (device + JSON object, e.g. `{"state":"CLOSE"}`).

## For every account
- **`/automation/scenes`** (the "Scenes" link on the devices page): the active scenes, one card each with its commands in order and an **Activate** button. Feedback appears next to the button: `✓ Scene activated`, or `✗` followed by each device whose command did not go out and why (not found or inactive, refused, source unavailable).
- **Scene widget** on the dashboard: the name, the description and the same button. A deleted or disabled scene is announced in the widget.
- **Activation** = publish each command on `<prefix>/<device name>/set`, **in order**, QoS 1. A failing command does not stop the following ones: failures are reported at the end. Any signed-in account may activate(the family runs "Evening closing").

## For administrators
- **`/admin/scenes`**: filterable list, add, edit, delete with confirmation. The command editor is a list of rows (device chosen from a dropdown, JSON command) with **↑ ↓** to reorder, **×** to remove and "Add a command". Errors are reported on the field: empty or taken name, command that is not an accepted JSON object (non-empty object, bounded depth and size), unknown device, more than 100 commands.
- **Widgets** (`/admin/widgets`): a widget's scene is picked from a dropdown (no number to know).
- Deleting a scene deletes its commands; widgets that pointed at it lose their scene, and automations that activated it fail (written to the [automation log](automations.md)).

## Notable behaviors
- A scene's **name is unique** (the script API activates a scene by name).
- A command to an **inactive** device is reported as failed (`not found or inactive`) instead of being published.
- A disabled scene can no longer be activated (answer 404).

## For developers
- `dominet_server::scenes::{activate, activate_by_id, activate_by_name}`: the automation engine (lot 3) and the script API (lot 5) go through them. They rely on `control::send_command` (immediate refusal without a broker link).
- Server functions: `scenes/list`, `scenes/activate` (any signed-in account), `scenes-admin/{list,save,delete}` (administrators).
- The form uses the generic CRUD field type `FieldKind::Pairs` (list of choice + text pairs, form value = JSON `[["12","{…}"],…]`, choices supplied by the `Choices` context under the field name).

## Tests
`cargo test -p dominet --test ui scene` (permissions, field-by-field validation, page), `-p dominet-server --test scenes` (publication **in order** on the dev broker, a failure does not stop the rest, **the three production scenes**: 6 / 4 / 4 commands on the right topics), `-p dominet-store --test automation` (storage); E2E `scenes` (full run in Firefox).
