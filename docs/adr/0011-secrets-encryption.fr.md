# 0011 — Chiffrement des valeurs sensibles au repos


- Statut : accepté
- Date : 2026-11-01

## Contexte
Les réglages applicatifs contiennent des secrets (clé API Free Mobile aujourd'hui ; mots de passe de caméras, jetons
et identifiants SMTP ensuite). PostgreSQL est sauvegardé (`pg_dump`) et peut être copié : une valeur en clair dans un
dump ou une sauvegarde est une fuite. La ROADMAP (§3.1) demandait d'évaluer un chiffrement applicatif en R1.

## Options
1. Valeurs en clair, la base étant « privée » — simple, mais tout dump, sauvegarde ou accès en lecture seule expose les secrets.
2. Chiffrement du disque ou de PostgreSQL (TDE) — ne protège pas contre un dump logique ni contre un accès SQL.
3. **Chiffrement applicatif des valeurs sensibles** avec une clé maîtresse hors base — retenu.

## Décision
- **AES-256-GCM** (crate `aes-gcm` 0.11), clé maîtresse de 32 octets en base64 dans `DOMINET_MASTER_KEY` (fichier `.env`, jamais en base ni dans git). `dominet gen-master-key` en génère une.
- **Format stocké** : `v1:` + base64( nonce 96 bits ‖ chiffré ‖ étiquette ). Le préfixe de version permet de changer d'algorithme plus tard.
- **Donnée associée = nom du réglage** : un chiffré recopié dans la ligne d'un autre réglage ne se déchiffre pas (testé).
- **Nonce aléatoire** à chaque chiffrement : deux chiffrés du même texte diffèrent.
- **Quels réglages** : ceux dont la clé contient `KEY`, `PASSWORD`, `TOKEN` ou `SECRET`, marqués `is_secret` ; le domaine refuse d'enregistrer une clé sensible non marquée secrète.
- **Valeur vide = stockée vide**, sans chiffrement (rien à protéger) ; cela permet d'annuler un secret sans clé maîtresse.
- **Aucune restitution** : l'API renvoie `********` pour un secret renseigné ; un secret est en écriture seule. Valeurs jamais journalisées (seul le nom du réglage l'est).
- **Cache en mémoire** déchiffré pour l'usage interne (envoi de SMS, etc.) ; il disparaît avec le processus.
- **Écriture refusée sans clé maîtresse** (503 explicite) plutôt qu'enregistrée en clair. `seed` vérifie tout avant d'écrire : une erreur n'écrit rien.
- La clé est **validée au démarrage** (`Config`) : une clé de mauvaise longueur arrête le serveur avec un message clair.

## Conséquences
- **Perdre la clé maîtresse rend les secrets irrécupérables** (il faudra les saisir à nouveau). Elle doit être sauvegardée à part de la base. Changer la clé rend les secrets existants illisibles ; ils sont signalés (`readable = false`, erreur 503 à l'usage) et les autres réglages continuent de fonctionner.
- Pas de rotation automatique : pour changer de clé, relire les secrets avec l'ancienne, puis les réécrire avec la nouvelle (outil à ajouter si le besoin apparaît).
- Le chiffrement protège la base **au repos** ; un attaquant qui contrôle le processus serveur (et donc la clé en mémoire) n'est pas couvert.
- Le même mécanisme servira aux mots de passe des caméras (R6).
