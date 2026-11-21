# Alertes

Règles, évaluation, page des alertes et pastille de la barre latérale (R4 lot 6). Décisions : [ADR 0035](adr/0035-alerts.md), plan R4 (D8, D10, D12).

Une **règle** surveille une clé de l'état d'un appareil (`temperature`, `state`, `power`…) ; quand sa condition est vraie, une **alerte** est créée. Elle reste « en cours » jusqu'à ce que la condition cesse (« résolue ») ou qu'un compte l'**acquitte**.

## Pour tous les comptes
- **`/alerts`** (lien « Alertes » de la barre latérale, avec une **pastille** du nombre d'alertes en cours, mise à jour en direct) : les alertes les plus récentes (200) filtrées par statut — en cours (par défaut), résolues, acquittées, toutes. Chaque ligne donne la sévérité, la règle, l'appareil, la valeur qui a déclenché, la date, le statut (et qui a acquitté, quand). Boutons **Acquitter** (une alerte en cours) et **Tout acquitter**.
- Chaque alerte déclenchée affiche un **bandeau** dans l'interface de tous les comptes connectés : `[DomiNet] Canicule — meteo_locale : 36.2` (niveau selon la sévérité).

## Alertes de la plateforme (Prometheus et Grafana)
La pastille de la barre latérale compte aussi les alertes **en cours** de Prometheus et de Grafana (application hors ligne, erreurs 5xx, MQTT déconnecté, canicule…), lues toutes les minutes en lecture seule : elles n'ont pas de bouton « Acquitter » et disparaissent quand leur cause disparaît. Aucun réglage de plus (`PROMETHEUS_URL`, `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD`). [ADR 0061](adr/0061-platform-alerts-in-the-alerts-page.md). La page `/alerts` les liste dans une section **Plateforme** (sévérité, source, nom, résumé, depuis quand), au-dessus des alertes d'appareil, pour les filtres « En cours » et « Toutes ». Si Prometheus ou Grafana ne répond pas, la page le dit (« Prometheus est injoignable ») au lieu d'afficher « aucune alerte ». Sans adresse configurée pour l'un ni l'autre, la section n'apparaît pas.

## Pour les administrateurs
- **`/admin/alert-rules`** : appareil, clé d'état, opérateur (`>`, `≥`, `<`, `≤`, `=`, `≠`, contient), seuil numérique (virgule admise) ou texte, sévérité (info, avertissement, critique), délai entre deux alertes (0 à 1 440 min), trois canaux (e-mail, SMS, Android), activation. Une modification est prise en compte tout de suite. Supprimer une règle laisse ses alertes.

## Comment une règle s'évalue
- À **chaque état** de l'appareil (Zigbee2MQTT ou appareil automatique). Clé absente ou `null` : on passe, sans rien déclencher ni résoudre.
- **Condition** : nombres (texte numérique et booléens compris : `"40"` ou `true` sont lus comme des nombres) ; si la valeur ou le seuil n'est pas un nombre, `=` et `≠` comparent les textes sans casse ni espaces et les autres opérateurs sont faux ; « contient » cherche le seuil texte dans le texte de la valeur, sans casse.
- **Vraie** : une alerte est créée, sauf si une alerte **en cours** de la règle date de moins que le délai. **Faux** : les alertes en cours de la règle sont résolues.
- Particularités reprises exprès : une alerte **acquittée** ne compte plus dans le délai (l'état chaud suivant en crée une aussitôt) ; une condition qui reste vraie recrée une alerte à chaque expiration du délai ; un délai de 0 crée une alerte à chaque état vrai.

## Envoi
Chaque alerte déclenchée affiche d'abord un **bandeau** dans l'interface, puis une tâche de fond sert les **canaux de la règle** (R5 lot 6, [ADR 0042](adr/0042-alerts-on-real-channels.md)) :

| Canal | Ce qui part |
|---|---|
| **E-mail** | au destinataire des alertes (`EMAIL_RECIPIENT`, à défaut l'expéditeur), sujet `[DomiNet Alerte <sévérité>] <règle>`, texte long (règle, appareil, valeur, date) |
| **SMS** | SMS Free Mobile, texte court (160 caractères au plus) ; ignoré sans identifiants `SMS_FREE_USER` / `SMS_FREE_KEY` |
| **Android** | `alert_notification` poussé à **tous** les appareils Android en ligne. L'application actuelle **ignore** ce message (« Message ignoré ») : il est envoyé pour une version future |

Chaque canal est tenté même si un autre échoue (les échecs sont dans le journal du serveur, jamais le secret) ; un canal non configuré est simplement ignoré. L'alerte passe à `notified` après l'essai, **échecs ou non** et n'est pas renvoyée : le délai de répétition de la règle décide du prochain essai. Les canaux et leur réglage : [notifications](notifications.fr.md). Rien n'est envoyé « pour de vrai » par les tests ni par la démonstration.

## Pour les développeurs
- `dominet_automation::alerts` : `AlertEngine`, `AlertHandle`, `Notifier`, `AlertNotice` ; branché dans `dominet_server::mqtt::spawn` (`AlertNotifier` dans `dominet_server::alerts`). Domaine : `AlertRuleDraft::evaluate`, `alert_messages`.
- Sujet temps réel `alerts` (`WsServerMsg::AlertCount`) : dernier état conservé ; publié au démarrage et à chaque changement.
- Fonctions serveur : `alerts/{page,acknowledge,acknowledge-all}` (tout compte connecté), `alert-rules/{list,save,delete}` (administrateurs).

## Tests
`cargo test -p dominet-domain --test alerts_oracle` (**2 030 évaluations comparées à des valeurs de référence figées**), `-p dominet-automation --test alerts` (**8 scénarios du cycle des alertes comparés à des valeurs de référence après chaque étape**, textes, redémarrage, règles éteintes), `-p dominet --test ui alert` (droits, validation champ par champ, page, pastille), `-p dominet-server --test alerts` (la règle canicule de la production de bout en bout sur le broker de dev) ; E2E `alerts`.
