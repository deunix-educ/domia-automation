# Rhai scripts

Automation scripts (`script` action) and handler code of MQTT sources (R4 lot 5). Decisions: [ADR 0006](adr/0006-rhai-sandbox.md) (spike), [ADR 0034](adr/0034-rhai-scripts.md) (implementation).

Scripts are written in **[Rhai](https://rhai.rs)**, an embedded language close to Rust/JavaScript. **Python** code does not port as is: it is refused when saving with a clear message (`def`, `import`, trailing colons, `context.`…).

## What a script sees
| Name | Content |
|---|---|
| `topic` | topic of the message or state that triggered the script (empty for a scheduled or solar trigger) |
| `payload` | the message: object (`payload.temperature`) or text; a missing key is `()` (never an error) |
| `device` | the device concerned or `()`; fields below |

| Device field | Content |
|---|---|
| `name`, `id`, `type` | name, number and type (`light`, `cover`, `sensor`…) |
| `state` | last known state (`device.state.position`) |
| `available` | `true` if the device is reachable |
| `topic` | topic of its state (`zigbee2mqtt/Salon`); append `"/set"` to command it |
| `age_secs` | age of its last message in seconds (decimal number, frozen when the script starts), or `()` if it never spoke |

| Function | Effect |
|---|---|
| `publish(topic, message)` | publishes on the broker (QoS 1, not retained). Text is published as is; a map (`#{ state: "ON" }`) or a number as JSON. No wildcard in the topic, 256 characters at most. Alias: `mqtt_publish` |
| `scene(name)` | activates the scene of that name |
| `get_device(name)` | reads a device (same fields as `device`), or `()` if it does not exist |
| `log_value(key, value)` | records a value of the device concerned, tracked or not (no effect without a device) |
| `print(text)`, `log(text)` | line of the execution log (50 lines, 500 characters) |

## Sandbox rules
- **No direct effect.** `publish`, `scene` and `log_value` are set aside and applied **in order after the script ends**, and **only if it ends without error** (all or nothing). At most 100 effects per run.
- **Limits**: 200,000 operations, 250 ms, 24 call levels, expressions 48 levels deep, 16 KB strings, 2,000-element arrays, 500-key maps. No `eval`, no `import`. An infinite loop, endless recursion or giant allocation is interrupted **without affecting the process**: the server keeps answering, the error is written to the automation's log.
- Each run happens in a separate thread (a panic there is an error).
- A script automation counts toward the guard of 30 firings per minute.

## Where to write a script
- **Automation**: `/admin/automations`, "Script" action. **MQTT source**: `/admin/mqtt-sources`, "Handler code" field (variables `topic` and `payload`, same functions; no execution log, the result goes to the server log; a failing code is muted for 10 s).
- Syntax is checked when saving: the parser's error (line, column) shows under the field.
- **Dry run** (button under the code area, administrators): runs the script with a sample topic and message against the real device states and shows what it would have done — **without publishing, activating or recording anything**.

## Examples
```rhai
// Turn the living room on above 25 degrees, and keep the value
if payload.temperature > 25.0 {
    publish("zigbee2mqtt/Salon/set", #{ state: "ON" });
}
log_value("temperature_bureau", payload.temperature);
```

```rhai
// Open the blind only if the door is closed
let porte = get_device("Porte");
if porte != () && porte.state.contact == true {
    publish("zigbee2mqtt/Volet-salon/set", #{ state: "OPEN" });
}
```

## The two scripts of the SB-est shutter
The two `Volet-SB-est` scripts are **written in Rhai** (`crates/dominet-domain/data/volet-sb-est-*.rhai`, `device_state` trigger on the switch's `action`, `single_left` and `single_right`). The left button opens, the right one closes, and a press during travel stops the shutter.

The real relay (`Relais-SB-est`) has **neither a `moving` key nor a `STOP` state**: its `state` derives from the position. It publishes about one message per second while moving, then goes silent; the script therefore treats it as **moving if its last message is less than 2 seconds old** (`age_secs`):
```rhai
let volet = get_device("Relais-SB-est");
if volet == () {
    throw "appareil Relais-SB-est introuvable";
}
let commande = volet.topic + "/set";
if volet.age_secs != () && volet.age_secs < 2.0 {
    publish(commande, #{ state: "STOP" });
} else {
    publish(commande, #{ state: "OPEN" });   // "CLOSE" for the right button
}
```

To use them: create two "device state" automations on the switch (key `action`, value `single_left` then `single_right`), "Script" action, and paste this code (adapting the relay's name). They replace the two simple "open" and "close" automations.

## Tests
`cargo test -p dominet-automation --lib script` (API, effects in order, all or nothing, **hostile scripts**: loops, recursion, giant strings / arrays / maps, `pad`, `eval`, nesting, endless effects; time limit alone; another language refused), `--test engine` (scripts in the engine, production plugins, muted source code), `-p dominet --test ui script` (dry run, syntax, permissions), `-p dominet-server --test automations` (end to end on the dev broker); E2E `automations` (editor, dry run, execution, infinite loop while the server keeps answering).
