# 0001 — Architecture globale


- Statut : accepté
- Date : 2026-10-31

## Contexte
DomiNet réécrit DomiNet (implémentation précédente) en Rust. Le cadrage complet est dans `ROADMAP-rust.md`
(§1 décisions, §2 architecture cible, §3 données). Cet ADR en fixe la forme et consigne les
ajustements décidés après les huit spikes de R0 (ADR 0002, 0003, 0005 à 0010).

## Décision
- **Un binaire `dominet`** (Axum + Tokio) avec sous-commandes et rôles (`--only web|mqtt|scheduler|video|publishers|discovery`) ; tâches supervisées avec redémarrage et backoff ; `/healthz`, `/readyz`, `/metrics`. Seul le rôle `discovery` est déployé à part (réseau hôte, multicast WS-Discovery).
- **Workspace Cargo** de 10 crates (`domain` ← `automation`/`surveillance`/`notify`/`integrations` ← `store`/`mqtt` ← `server` ; `proto` partagé serveur/WASM ; `web` pour Leptos). `domain` sans I/O ni framework.
- **Front** : Leptos 0.8 SSR + hydratation (ADR 0007). **Données** : PostgreSQL (`sqlx` 0.9) pour configuration, utilisateurs, agrégats et état récent ; ReductStore pour vidéo, snapshots, détections et IoT brut (ADR 0002). **Bus** : MQTT `rumqttc` (ADR 0005) + WebSocket Axum. **Live** : WHEP via MediaMTX (ADR 0010).
- **Scripts utilisateur** : Rhai sandboxé, sans effet de bord direct (ADR 0006). **Vision** : `ffmpeg` en sous-processus derrière `FrameSource` (ADR 0003), YOLO via `ort` (ADR 0009), ONVIF maison (ADR 0008).
- **Authentification** : ADR 0004.
- **Déploiement** : Docker Compose (voie officielle) et binaire release ; **cible principale x86_64** (EliteDesk i5-6500), arm64 au mieux. Image **glibc** (Debian slim) : ONNX Runtime n'a pas de binaire musl. Binaires publiés au niveau de base **`x86-64-v2`** (jamais `target-cpu=native`) ; le calcul lourd (ONNX Runtime) choisit lui-même ses noyaux AVX2 à l'exécution.
- **Observabilité** : `tracing` structuré, `/metrics` Prometheus, Grafana conservé.

## Ajustements issus de R0
- Stockage : ReductStore = archive brute et requêtes à seuil ; **tous les graphiques** viennent de PostgreSQL.
- `ort` n'a que des versions candidates : version épinglée exactement, test de parité obligatoire à chaque montée.
- Leptos reste en 0.8 (la 0.9.0-rc1 est sortie le 2026-10-31 ; `leptos-fluent` exige `< 0.9`).
- Docker : le service `dominet` reçoit `/dev/dri` et le groupe `render` (VAAPI) ; MediaMTX publie son port ICE **sans décalage** ; ReductStore exige un jeton non vide.
- ONVIF : l'ajout manuel par IP est obligatoire (certaines caméras ne répondent pas au multicast) ; OpenIPC n'a pas d'ONVIF.

## Conséquences
- Le chemin critique reste R1 → R2 → R3 → R4 → R7 → R8 ; R5 et R6 peuvent avancer en parallèle après R2.
- Toute entorse à ces choix passe par un nouvel ADR qui remplace celui-ci.
