# 0042 — Alertes et notifications d'automation sur les canaux réels


- Statut : accepté
- Date : 2026-11-07

## Contexte
R4 (ADR 0035) a posé le port `Notifier` : une alerte déclenchée produit un bandeau, ses trois canaux (e-mail, SMS, Android) sont enregistrés mais pas servis, `notified` reste faux. R5 a livré les canaux (ADR 0038) et le relais Android (ADR 0039). Plan R5 : D7, D8.

## Décision
- **Alertes (D8)** : `AlertNotifier` (`dominet_server::alerts`) remplace `UiNotifier`. Sur `fired` : le bandeau part tout de suite, puis une tâche de fond sert les canaux **de la règle** — e-mail au destinataire des alertes (`EMAIL_RECIPIENT`, à défaut l'expéditeur) avec le sujet `[DomiNet Alerte <sévérité>] <règle>` et le texte long de la référence ; SMS Free Mobile avec le texte court (160 caractères) ; push `alert_notification` à **tous** les appareils Android en ligne. L'évaluation des alertes n'attend jamais un envoi.
- **Isolation** : chaque canal est tenté même si un autre échoue ; un canal **non configuré** (pas de SMTP, pas d'identifiants Free Mobile) est ignoré avec un message d'information, un échec réel est journalisé (jamais le secret). `notified` passe à vrai après l'essai, échecs ou non (comme la référence) ; les échecs ne sont pas relancés (le délai de répétition de la règle s'en charge).
- **Push Android conservé** bien que l'application actuelle l'ignore (« Message ignoré ») : il est compatible avec une future version, et l'interface des règles le dit (indication sous la case).
- **Code HTTP de Free Mobile vérifié** (la référence l'ignorait) : un refus est un échec journalisé.
- **Notification d'automation (D7)** : l'action `notification` garde le bandeau et envoie en plus **un e-mail** au destinataire des alertes (sujet `[DomiNet] Notification`) et, **si l'automation coche « SMS »**, un SMS Free Mobile (`Channels::notify`, l'intention de `notify()` de la référence, que l'ancien code n'appelait pas depuis les automations). Un canal qui échoue n'empêche pas l'autre.
- **Aucun envoi réel** (D1) : tests et démonstration utilisent le faux SMTP, le faux Free Mobile et un téléphone simulé.

## À la bascule (R8)
Les automations de la production qui ont une action `notification` (et les alertes dont les cases e-mail / SMS sont cochées) enverront **vraiment** des messages dès que le serveur Rust sera branché sur le vrai SMTP et les vrais identifiants : à vérifier avant la bascule (`dominet notify-test`, lancé par vous).

## Vérification
- `crates/dominet/tests/alert_channels.rs` (5 tests) : le **vrai moteur d'alertes** déclenche, l'e-mail (expéditeur, destinataire, sujet, texte), le SMS (identifiants, texte court) et le push reçu par un téléphone connecté sont exactement ceux attendus ; seuls les canaux de la règle servent ; e-mail refusé : le SMS part et `notified` passe à vrai ; canal non configuré ignoré ; notification d'automation (SMS seulement si demandé, échec rapporté).
- E2E : e-mail d'alerte reçu par le SMTP simulé et alerte marquée notifiée, sur l'image Docker.

## Conséquences
- Une alerte dont l'envoi échoue n'est pas rejouée : seul le prochain déclenchement (après le délai de la règle) retentera.
- `Channels` est dans `AppState` : toute future notification (R7, alertes Prometheus) l'utilise telle quelle.
