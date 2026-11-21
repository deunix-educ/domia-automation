#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""E2E des automations dans Firefox (R4 lot 3), sur le broker de dev : administration (formulaire selon le type, JSON refuse s'il est
invalide), declencheur `device_state` (etat publie -> commande publiee), declencheur `mqtt_message` sur un topic sans source (filtre,
condition, notification affichee dans le navigateur), journal d'execution, garde-fou anti-boucle (30 par minute), planificateur
(cron chaque minute, coucher du soleil, prochain passage affiche), scripts Rhai (syntaxe, essai a blanc, execution, boucle infinie), desactivation et suppression prises en compte a chaud.

Usage : python3 tests/e2e/automations.py url utilisateur_admin mot_de_passe
Variables : E2E_DB_SHELL (SQL sur la base jetable). Le prefixe Zigbee2MQTT est remplace par un prefixe propre au script.
"""
import json
import os
import subprocess
import sys
import time
import urllib.parse

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from marionette import Marionette, Report  # noqa: E402

if len(sys.argv) < 4:
    sys.exit("usage: automations.py url utilisateur_admin mot_de_passe")
url, admin, password = sys.argv[1], sys.argv[2], sys.argv[3]
db_shell = os.environ.get("E2E_DB_SHELL")
if not db_shell:
    sys.exit("E2E_DB_SHELL absent : lancer par tools/e2e.sh")
broker = os.environ.get("E2E_MQTT_CONTAINER", "dominet-dev-mosquitto-1")
stamp = int(time.time())
z2m = f"e2eautoz{stamp}"        # base Zigbee2MQTT du serveur pendant ce script
prefix = f"e2eauto{stamp}"      # topics de commande des appareils et topic surveille
remote, blind = "E2E Remote", "E2E Blind"

m = Marionette()
r = Report()


def sql(text):
    out = subprocess.run(db_shell, shell=True, input=text.encode(), check=True, capture_output=True)
    return out.stdout.decode()


def scalar(query):
    return sql(f"COPY ({query}) TO STDOUT;").strip()


def pub(topic, payload):
    body = payload if isinstance(payload, str) else json.dumps(payload)
    subprocess.run(["docker", "exec", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", topic, "-m", body],
                   check=True, capture_output=True, stdin=subprocess.DEVNULL)


def pub_many(topic, payloads):
    """Publie plusieurs messages d'un coup (une ligne = un message)."""
    subprocess.run(["docker", "exec", "-i", broker, "mosquitto_pub", "-h", "localhost", "-q", "1", "-t", topic, "-l"],
                   check=True, capture_output=True, input="\n".join(payloads).encode())


def listen(topic, count=1):
    return subprocess.Popen(["docker", "exec", broker, "mosquitto_sub", "-h", "localhost", "-q", "1", "-v", "-C", str(count), "-W", "20", "-t", topic],
                            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL, text=True)


def heard(proc):
    try:
        out, _ = proc.communicate(timeout=25)
    except subprocess.TimeoutExpired:
        proc.kill()
        return []
    return [line for line in out.strip().splitlines() if line]


def wait_until(predicate, seconds=15):
    end = time.time() + seconds
    while time.time() < end:
        if predicate():
            return True
        time.sleep(0.3)
    return False


def fill(selector, value):
    return m.js(
        f"const e = document.querySelector({selector!r}); if (!e) return false; e.focus(); e.value = {value!r};"
        "e.dispatchEvent(new Event('input', {bubbles: true})); return true;"
    )


def choose(selector, label_or_value):
    return m.js(
        f"const s = document.querySelector({selector!r}); if (!s) return false;"
        f"const o = [...s.options].find(o => o.value === {label_or_value!r} || o.textContent === {label_or_value!r}); if (!o) return false;"
        "s.value = o.value; s.dispatchEvent(new Event('change', {bubbles: true})); return true;"
    )


def check_box(selector, on):
    return m.js(f"const e = document.querySelector({selector!r}); if (!e) return false; if (e.checked !== {str(on).lower()}) e.click(); return true;")


def post(path, body):
    return json.loads(m.js_async(
        "const cb = arguments[arguments.length-1];"
        f"fetch({path!r}, {{method:'POST', headers:{{'Content-Type':'application/x-www-form-urlencoded'}}, body:{body!r}}})"
        ".then(async x => cb(JSON.stringify({status: x.status, text: await x.text()}))).catch(e => cb(JSON.stringify({status: 0, text: String(e)})));"
    ))


def open_form():
    for _ in range(40):  # tant que l'interface n'est pas hydratee, le clic n'ouvre rien
        m.click("#crud-add")
        if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
            return True
    return False


def save_enabled():
    return m.wait_for("document.querySelector('#crud-save') && !document.querySelector('#crud-save').disabled", 15)


def count(name):
    return scalar(f"SELECT trigger_count FROM automation WHERE name = '{name}'")


def edit_row(name):
    m.js(f"[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.textContent.includes({name!r})).querySelector('.crud-edit').click()")


def journal(name):
    return scalar(f"SELECT string_agg(status || '|' || message, ';' ORDER BY id) FROM automation_run WHERE automation_id = (SELECT id FROM automation WHERE name = '{name}')")


m.goto(url)
m.cmd("WebDriver:DeleteAllCookies")
m.js("localStorage.clear()")
res = m.js_async(
    "const cb = arguments[arguments.length-1];"
    "fetch('/api/auth/login', {method:'POST', headers:{'Content-Type':'application/json'},"
    f"body: JSON.stringify({{username:{admin!r}, password:{password!r}}})}}).then(x => cb(x.status));"
)
r.check("connexion administrateur", res == 200, str(res))
res = post("/sf/settings/save", "key=Z2M_BASE_TOPIC&value=" + urllib.parse.quote(z2m, safe=""))
r.check("prefixe Zigbee2MQTT propre au script", res["status"] == 200 and "Saved" in res["text"], res["text"][:120])
sql(f"INSERT INTO device (friendly_name, base_topic) VALUES ('{remote}', '{z2m}'), ('{blind}', '{prefix}');")
time.sleep(1.5)

# --- formulaire : champs selon le type ------------------------------------------------------------------------------------------
m.goto(url + "admin/automations")
r.check("page des automations", m.wait_for("document.querySelector('#crud-add') !== null", 10))
open_form()
save_enabled()
has = lambda f: m.js(f"return document.querySelector('#crud-field-{f}') !== null")
r.check("declencheur par defaut : etat d'appareil", has("friendly_name") and has("key") and has("value") and not has("topic") and not has("cron"))
choose("#crud-field-trigger_type", "mqtt_message")
r.check("message MQTT : topic et filtre", m.wait_for("document.querySelector('#crud-field-topic') !== null && document.querySelector('#crud-field-payload_filter') !== null", 5) and not has("friendly_name"))
choose("#crud-field-trigger_type", "schedule")
r.check("horaire : expression cron", m.wait_for("document.querySelector('#crud-field-cron') !== null", 5) and not has("topic"))
choose("#crud-field-trigger_type", "solar")
r.check("solaire : evenement, latitude, longitude", m.wait_for("document.querySelector('#crud-field-event') !== null", 5) and has("latitude") and has("longitude"))
choose("#crud-field-trigger_type", "device_state")
r.check("la liste des appareils vient de la base", m.wait_for(f"[...(document.querySelector('#crud-field-friendly_name')?.options || [])].some(o => o.textContent === {remote!r})", 10))
choose("#crud-field-action_type", "scene")
r.check("action scene : liste des scenes", m.wait_for("document.querySelector('#crud-field-scene_id') !== null", 5) and not has("action_topic"))
choose("#crud-field-action_type", "notification")
r.check("action notification : message", m.wait_for("document.querySelector('#crud-field-message') !== null", 5))
choose("#crud-field-action_type", "script")
r.check("action script : zone de code", m.wait_for("document.querySelector('#crud-field-script') !== null", 5))
choose("#crud-field-action_type", "mqtt_publish")
m.wait_for("document.querySelector('#crud-field-action_topic') !== null", 5)

# --- automation 1 : etat de la telecommande -> commande du volet --------------------------------------------------------------------
fill("#crud-field-name", "E2E TC ouvrir")
choose("#crud-field-friendly_name", remote)
fill("#crud-field-key", "action")
fill("#crud-field-value", "1_single")
fill("#crud-field-action_topic", f"{prefix}/{blind}/set")
fill("#crud-field-action_payload", "pas du json")
m.click("#crud-save")
r.check("JSON invalide : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-action_payload') !== null", 10), m.text("#crud-error-action_payload"))
r.check("... et rien n'est enregistre", scalar("SELECT count(*) FROM automation WHERE name = 'E2E TC ouvrir'") == "0")
fill("#crud-field-action_payload", '{"state":"OPEN"}')
m.click("#crud-save")
r.check("automation creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E TC ouvrir')", 10))
r.check("... avec le JSON de reference en base",
        json.loads(scalar("SELECT trigger_config::text || '|' || action_config::text FROM automation WHERE name = 'E2E TC ouvrir'").split("|")[0]) ==
        {"friendly_name": remote, "key": "action", "value": "1_single"})
time.sleep(1.0)  # le moteur recharge ses automations a l'enregistrement

sub = listen(f"{prefix}/#")
time.sleep(1.0)
pub(f"{z2m}/{remote}", {"action": "1_single", "battery": 97})
got = heard(sub)
r.check("etat publie -> commande du volet publiee", got == [f'{prefix}/{blind}/set {{"state":"OPEN"}}'], str(got))
r.check("compteur de declenchements en base", wait_until(lambda: count("E2E TC ouvrir") == "1"), count("E2E TC ouvrir"))
r.check("journal : ok", (journal("E2E TC ouvrir") or "").startswith("ok|published on "), journal("E2E TC ouvrir"))

# --- automation 2 : message MQTT sur un topic sans source, filtre + condition -> notification --------------------------------------------
open_form()
save_enabled()
choose("#crud-field-trigger_type", "mqtt_message")
m.wait_for("document.querySelector('#crud-field-topic') !== null", 5)
fill("#crud-field-name", "E2E Bouton")
fill("#crud-field-topic", f"{prefix}in/+")
fill("#crud-field-payload_filter", '{"action": "single"}')
fill("#crud-field-condition_key", "battery")
fill("#crud-field-condition_value", "100")
choose("#crud-field-action_type", "notification")
m.wait_for("document.querySelector('#crud-field-message') !== null", 5)
fill("#crud-field-message", "Bouton E2E appuye")
m.click("#crud-save")
r.check("automation message creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Bouton')", 10))
time.sleep(1.5)  # rechargement du moteur et nouvel abonnement du client MQTT
m.wait_for("document.querySelector('#vc-rt-state')?.dataset.state === 'online'", 20)
pub(f"{prefix}in/salon", {"action": "double", "battery": 100})     # filtre
pub(f"{prefix}in/salon", {"action": "single", "battery": 50})      # condition fausse
time.sleep(1.0)
r.check("filtre et condition : rien ne se declenche", count("E2E Bouton") == "0")
pub(f"{prefix}in/salon", {"action": "single", "battery": 100})
r.check("topic sans source : l'automation se declenche", wait_until(lambda: count("E2E Bouton") == "1"), count("E2E Bouton"))
r.check("notification affichee dans le navigateur", m.wait_for("[...document.querySelectorAll('.vc-toast')].some(t => t.textContent.includes('Bouton E2E appuye'))", 15))
r.check("journal : notification", (journal("E2E Bouton") or "") == "ok|notification : Bouton E2E appuye", journal("E2E Bouton"))

# --- journal visible dans la page ---------------------------------------------------------------------------------------------------
m.click("#runs-refresh")
r.check("le journal de la page liste les executions",
        m.wait_for("document.querySelectorAll('#runs-table tbody tr').length >= 2", 10), str(m.js("return document.querySelectorAll('#runs-table tbody tr').length")))
r.check("... avec le nom, le declencheur et le resultat",
        m.js("return [...document.querySelectorAll('#runs-table tbody tr')].some(t => t.textContent.includes('E2E TC ouvrir') && t.textContent.includes('state E2E Remote: action = 1_single') && t.querySelector('.vc-badge--success'))"))

# --- garde-fou anti-boucle ---------------------------------------------------------------------------------------------------------------
before = int(count("E2E Bouton"))
pub_many(f"{prefix}in/boucle", [json.dumps({"action": "single", "battery": 100})] * 40)
r.check("garde-fou : 30 declenchements par minute au plus", wait_until(lambda: int(count("E2E Bouton")) >= 30, 20) and int(count("E2E Bouton")) <= 30,
        f"avant {before}, apres {count('E2E Bouton')}")
time.sleep(1.5)
r.check("... et le depassement est inscrit une seule fois au journal",
        scalar("SELECT count(*) FROM automation_run WHERE status = 'limited' AND automation_id = (SELECT id FROM automation WHERE name = 'E2E Bouton')") == "1")

# --- scripts Rhai : syntaxe, essai a blanc, execution, script hostile ------------------------------------------------------------------
open_form()
save_enabled()
choose("#crud-field-trigger_type", "mqtt_message")
m.wait_for("document.querySelector('#crud-field-topic') !== null", 5)
fill("#crud-field-name", "E2E Script")
fill("#crud-field-topic", f"{prefix}sc/+")
choose("#crud-field-action_type", "script")
r.check("action script : zone de code et panneau d'essai a blanc", m.wait_for("document.querySelector('#crud-field-script') !== null && document.querySelector('#dryrun-run') !== null", 5))
fill("#crud-field-script", "let x = ;")
m.click("#crud-save")
r.check("script qui ne se compile pas : erreur de syntaxe sur le champ", m.wait_for("(document.querySelector('#crud-error-script')?.textContent || '').includes('Script invalide')", 10), m.text("#crud-error-script"))
fill("#crud-field-script", "def f():\n  pass")
m.click("#crud-save")
r.check("code Python refuse avec un message clair", m.wait_for("(document.querySelector('#crud-error-script')?.textContent || '').includes('Python')", 10), m.text("#crud-error-script"))
code = f'let c = payload.n * 2; publish("{prefix}/script/set", #{{ double: c }}); print("ok");'
fill("#crud-field-script", code)
m.wait_for("document.querySelector('#dryrun-run') && !document.querySelector('#dryrun-run').disabled", 10)
fill("#dryrun-topic", "essai/x")
fill("#dryrun-payload", '{"n": 4}')
m.click("#dryrun-run")
r.check("essai a blanc : ce que le script ferait", m.wait_for(f"(document.querySelector('#dryrun-effects')?.textContent || '').includes('publish on {prefix}/script/set: {{\"double\":8}}')", 15), m.text("#dryrun-result"))
r.check("... et son journal", (m.text("#dryrun-logs") or "") == "ok", m.text("#dryrun-logs"))
r.check("... sans rien publier ni enregistrer", scalar("SELECT count(*) FROM automation WHERE name = 'E2E Script'") == "0")
m.click("#crud-save")
r.check("automation script creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Script')", 10))
time.sleep(1.5)
sub = listen(f"{prefix}/script/#")
time.sleep(1.0)
pub(f"{prefix}sc/x", {"n": 5})
got = heard(sub)
r.check("le script s'execute sur le message et publie", got == [f'{prefix}/script/set {{"double":10}}'], str(got))
r.check("journal : script ok", wait_until(lambda: (journal("E2E Script") or "").startswith("ok|script: 1 effect(s)")), journal("E2E Script"))

open_form()
save_enabled()
choose("#crud-field-trigger_type", "mqtt_message")
m.wait_for("document.querySelector('#crud-field-topic') !== null", 5)
fill("#crud-field-name", "E2E Hostile")
fill("#crud-field-topic", f"{prefix}sc/hostile")
choose("#crud-field-action_type", "script")
m.wait_for("document.querySelector('#crud-field-script') !== null", 5)
fill("#crud-field-script", 'publish("e2e/jamais", 1); loop { }')
m.wait_for("document.querySelector('#dryrun-run') && !document.querySelector('#dryrun-run').disabled", 10)
t0 = time.time()
m.click("#dryrun-run")
r.check("essai a blanc d'une boucle infinie : interrompu, erreur affichee",
        m.wait_for("(document.querySelector('#dryrun-error')?.textContent || '').length > 0", 15) and time.time() - t0 < 10, m.text("#dryrun-error"))
r.check("... aucun effet annonce", m.js("return document.querySelectorAll('#dryrun-effects li').length") == 0)
m.click("#crud-save")
r.check("automation hostile creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Hostile')", 10))
time.sleep(1.5)
sub = listen("e2e/jamais")
pub(f"{prefix}sc/hostile", {"n": 1})
served = [m.js_async("const cb = arguments[arguments.length-1]; fetch('/').then(x => cb(x.status)).catch(() => cb(0));") for _ in range(3)]
r.check("le serveur repond pendant et apres le script hostile", served == [200, 200, 200], str(served))
r.check("script hostile : journal en echec (interrompu)", wait_until(lambda: (journal("E2E Hostile") or "").startswith("error|script failed")), journal("E2E Hostile"))
r.check("... et sa publication n'est jamais partie", heard(sub) == [])
sub = listen(f"{prefix}/script/#")
time.sleep(1.0)
pub(f"{prefix}sc/x", {"n": 6})
got = heard(sub)
r.check("le moteur continue apres le script hostile", got == [f'{prefix}/script/set {{"double":12}}'], str(got))

# --- planificateur : cron chaque minute et coucher du soleil ----------------------------------------------------------------------------
open_form()
save_enabled()
choose("#crud-field-trigger_type", "schedule")
m.wait_for("document.querySelector('#crud-field-cron') !== null", 5)
fill("#crud-field-name", "E2E Minute")
fill("#crud-field-cron", "61 * * * *")
choose("#crud-field-action_type", "notification")
m.wait_for("document.querySelector('#crud-field-message') !== null", 5)
fill("#crud-field-message", "Top minute E2E")
m.click("#crud-save")
r.check("cron invalide : erreur sur le champ", m.wait_for("document.querySelector('#crud-error-cron') !== null", 10), m.text("#crud-error-cron"))
fill("#crud-field-cron", "* * * * *")
m.click("#crud-save")
r.check("automation horaire creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Minute')", 10))
open_form()
save_enabled()
choose("#crud-field-trigger_type", "solar")
m.wait_for("document.querySelector('#crud-field-event') !== null", 5)
fill("#crud-field-name", "E2E Soleil")
choose("#crud-field-event", "sunset")
fill("#crud-field-latitude", "43,3")
fill("#crud-field-longitude", "5,4")
choose("#crud-field-action_type", "notification")
m.wait_for("document.querySelector('#crud-field-message') !== null", 5)
fill("#crud-field-message", "Coucher E2E")
m.click("#crud-save")
r.check("automation solaire creee", m.wait_for("[...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === 'E2E Soleil')", 10))
r.check("un travail du planificateur par automation planifiee",
        wait_until(lambda: scalar("SELECT count(*) FROM schedule_job WHERE automation_id IS NOT NULL") == "2"), scalar("SELECT count(*) FROM schedule_job WHERE automation_id IS NOT NULL"))
r.check("le prochain passage de chacun est calcule",
        wait_until(lambda: scalar("SELECT count(*) FROM schedule_job WHERE automation_id IS NOT NULL AND next_run_at IS NOT NULL") == "2"))
r.check("le coucher du soleil est dans les 24 prochaines heures",
        scalar("SELECT bool_and(next_run_at > now() AND next_run_at < now() + interval '25 hours') FROM schedule_job WHERE solar_event = 'sunset'") == "t")
m.goto(url + "admin/automations")
r.check("le tableau affiche le prochain passage",
        m.wait_for("[...document.querySelectorAll('#crud-table tbody tr')].filter(t => t.textContent.includes('E2E Soleil') || t.textContent.includes('E2E Minute')).every(t => /20\\d\\d-\\d\\d-\\d\\d \\d\\d:\\d\\d:\\d\\d/.test(t.textContent))", 15))
r.check("le travail systeme du bilan d'energie existe (00:05 locale)",
        scalar("SELECT cron_expr || '|' || timezone FROM schedule_job WHERE name = 'compute_daily_energy'") == "5 0 * * *|Europe/Paris")
r.check("le cron chaque minute se declenche tout seul (moins de 90 s)",
        wait_until(lambda: scalar("SELECT count(*) FROM automation_run WHERE trigger = 'schedule * * * * *' AND status = 'ok'") != "0", 95))
r.check("... et notifie le navigateur", m.wait_for("[...document.querySelectorAll('.vc-toast')].some(t => t.textContent.includes('Top minute E2E'))", 15))
r.check("... le travail note son passage", scalar("SELECT last_status FROM schedule_job WHERE name LIKE 'automation-%' AND cron_expr = '* * * * *'").startswith("ok"))

# --- desactivation a chaud ----------------------------------------------------------------------------------------------------------------
m.goto(url + "admin/automations")
m.wait_for("document.querySelectorAll('#crud-table tbody tr').length >= 2", 10)
for _ in range(40):
    edit_row("E2E TC ouvrir")
    if m.wait_for("document.querySelector('#crud-form') !== null", 0.5):
        break
save_enabled()
r.check("formulaire d'edition prerempli", m.js("return document.querySelector('#crud-field-key').value") == "action"
        and m.js("return document.querySelector('#crud-field-action_payload').value") == '{"state":"OPEN"}')
check_box("#crud-field-enabled", False)
m.click("#crud-save")
r.check("automation desactivee", m.wait_for("document.querySelector('#crud-form') === null", 10) and scalar("SELECT enabled FROM automation WHERE name = 'E2E TC ouvrir'") == "f")
time.sleep(1.5)
pub(f"{z2m}/{remote}", {"action": "1_single", "battery": 96})
time.sleep(2.0)
r.check("desactivee : l'etat ne declenche plus rien", count("E2E TC ouvrir") == "1", count("E2E TC ouvrir"))

# --- suppression --------------------------------------------------------------------------------------------------------------------------------
for name in ("E2E TC ouvrir", "E2E Bouton", "E2E Script", "E2E Hostile", "E2E Minute", "E2E Soleil"):
    for _ in range(40):
        m.js(f"[...document.querySelectorAll('#crud-table tbody tr')].find(t => t.textContent.includes({name!r}))?.querySelector('.crud-delete')?.click()")
        if m.wait_for("document.querySelector('.crud-confirm-yes') !== null", 0.5):
            break
    m.click(".crud-confirm-yes")
    r.check(f"{name} supprimee (journal compris)", m.wait_for(f"![...document.querySelectorAll('#crud-table tbody tr td')].some(t => t.textContent === {name!r})", 10)
            and scalar(f"SELECT count(*) FROM automation WHERE name = '{name}'") == "0")
r.check("le journal des supprimees est vide en base", scalar("SELECT count(*) FROM automation_run") == "0")
r.check("leurs travaux planifies ont disparu", wait_until(lambda: scalar("SELECT count(*) FROM schedule_job WHERE automation_id IS NOT NULL") == "0"))

# Nettoyage : la base jetable est partagee avec les scripts suivants.
sql(f"DELETE FROM device WHERE friendly_name IN ('{remote}', '{blind}');")
sys.exit(r.finish())
