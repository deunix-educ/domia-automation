# Notifications : canaux d'envoi

Depuis R5 (lot 2, [ADR 0038](adr/0038-delivery-channels.md)), DomiNet sait envoyer des **e-mails** (SMTP) et des **SMS Free Mobile**. La crate `dominet-notify` ne connaît ni la base ni l'interface ; `dominet_server::notify::Channels` l'assemble avec la configuration.

## Où se règle quoi

| Quoi | Où | Clé |
|---|---|---|
| Serveur SMTP, port, identifiants, chiffrement | environnement (redémarrage) | `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS`, `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS` |
| Expéditeur | réglage (immédiat) | `DEFAULT_FROM_EMAIL` |
| Destinataire des alertes | réglage | `EMAIL_RECIPIENT` (à défaut : l'expéditeur) |
| Identifiants Free Mobile | réglages (la clé est chiffrée au repos) | `SMS_FREE_USER`, `SMS_FREE_KEY` |
| Adresse de l'API Free Mobile | environnement | `SMS_FREE_URL` (défaut : l'API officielle) |

## Sécurité

- `EMAIL_USE_TLS` (défaut) : `STARTTLS` **obligatoire**, jamais de repli en clair ; `EMAIL_USE_SSL` : TLS dès l'ouverture (port 465). Les deux ensemble sont refusés. Sans chiffrement, des identifiants ne sont admis que vers la boucle locale (la configuration est refusée au démarrage sinon).
- Le mot de passe SMTP et la clé Free Mobile n'apparaissent ni dans `Debug`, ni dans les journaux, ni dans les erreurs (la clé voyage dans l'adresse HTTP : les erreurs de `reqwest` sont écrites sans adresse, et les redirections sont refusées).
- Un saut de ligne dans un sujet est remplacé par une espace : pas d'injection d'en-tête.
- Le code HTTP de Free Mobile est vérifié (400, 402, 403, 500 expliqués) . Le message est coupé à 160 caractères, délai de 5 s.

## Essayer pour de vrai : `dominet notify-test`

```
dominet notify-test --email            # au destinataire des alertes
dominet notify-test --email --to moi@example.com
dominet notify-test --sms
```

**C'est la seule commande qui envoie vraiment** (avec les alertes, les notifications d'automation et la messagerie, qui envoient aussi vraiment dès que le SMTP est configuré) : elle ne se lance que sur votre demande. Les tests, la démonstration et le développement parlent à de faux serveurs (`dominet-notify`, feature `testing` : `FakeSmtp`, `FakeFreeMobile`) et ne sortent jamais de la machine.
