# 0038 — Canaux d'envoi : e-mail SMTP et SMS Free Mobile


- Statut : accepté
- Date : 2026-11-06

## Contexte
Les alertes, la messagerie et l'action d'automation `notification` partent par deux canaux : l'e-mail (SMTP, par exemple Gmail) et le SMS d'alerte par l'API Free Mobile, dont il faut vérifier le code de retour. R5 (plan, D1, D7, D8, D11) met en place ces deux canaux pour les alertes, la messagerie et l'action d'automation `notification`, sans jamais envoyer pour de vrai pendant le développement.

## Options
- **E-mail** : `lettre` (SMTP asynchrone, `rustls`) ; ou une commande `sendmail` ; ou un appel HTTP à un service tiers. `lettre` lit les variables d'environnement `EMAIL_*` usuelles et n'ajoute aucun service.
- **Tests** : un crate de simulation tiers, ou de petits faux serveurs maison sur la boucle locale. Un faux serveur maison enregistre exactement ce qui est reçu et sert aussi à la démonstration.
- **Où règle-t-on quoi** : tout en environnement, ou tout en réglages. Le serveur SMTP (hôte, mot de passe) est une affaire d'installation ; l'expéditeur, le destinataire et les identifiants Free Mobile existent déjà comme réglages modifiables sans redémarrage.

## Décision
- **Crate `dominet-notify`** (sans base ni interface) : `SmtpSender` (`lettre`, texte brut UTF-8, une connexion par message, délai configurable) et `FreeMobile` (`reqwest`, GET `?user&pass&msg`, 160 caractères, 5 s). Les adaptations au serveur vivent dans `dominet_server::notify::Channels`.
- **Environnement** : `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS` (défaut vrai), `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS` (10), `SMS_FREE_URL`. **Réglages** (déjà au registre) : `DEFAULT_FROM_EMAIL`, `EMAIL_RECIPIENT` (à défaut l'expéditeur, comme la référence), `SMS_FREE_USER`, `SMS_FREE_KEY` (secret chiffré).
- **Chiffrement sans repli** : `STARTTLS` obligatoire (une connexion qui ne l'obtient pas échoue, rien n'est envoyé en clair) ; TLS direct avec `EMAIL_USE_SSL` ; `TLS` et `SSL` ensemble sont une erreur de configuration. Des **identifiants sans TLS ne sont admis que vers la boucle locale** : la configuration est refusée au démarrage sinon.
- **Aucun secret en sortie** : `Debug` masque le mot de passe et la clé ; les erreurs HTTP sont écrites sans adresse (`without_url`, car la clé est dans l'adresse) ; les redirections sont refusées pour que la clé ne suive pas.
- **Écarts voulus avec la référence** : le code HTTP de Free Mobile est vérifié (400, 402, 403, 500 expliqués) ; un saut de ligne dans un sujet devient une espace.
- **Faux serveurs** (`dominet_notify::testing`, feature `testing`) : `FakeSmtp` (EHLO, AUTH PLAIN / LOGIN, MAIL / RCPT / DATA, refus configurables, pas de `STARTTLS`) et `FakeFreeMobile` (code HTTP configurable). Aucun test, ni la démonstration, ne contacte `smtp.gmail.com` ni `smsapi.free-mobile.fr`.
- **`dominet notify-test --email [--to] --sms`** : envoi réel d'un message d'essai, **seule voie réelle**, que l'utilisateur lance lui-même.

## Conséquences
- Le chiffrement `STARTTLS` / TLS n'est pas exercé par les tests (le faux serveur est en clair) : il l'est par `notify-test` chez l'utilisateur ; les tests vérifient seulement qu'il est exigé.
- Un changement d'`EMAIL_*` demande un redémarrage ; un changement de réglage est pris à l'envoi suivant (aucun cache).
- Les lots suivants (alertes, messagerie, action `notification`) appellent `Channels` ; le port `Notifier` d'R4 garde son interface.
