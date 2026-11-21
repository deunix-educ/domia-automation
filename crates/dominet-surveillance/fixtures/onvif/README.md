<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Reponses ONVIF de reference (R6 lot 3)

Reponses SOAP rejouees par le faux serveur ONVIF (`dominet_surveillance::testing::onvif`). **Reconstituees** d'apres ce que le
spike 6 a observe sur les vraies cameras (ADR 0008) : forme des reponses, chemins des services, jetons de profil, resolutions,
port 8899 de la camera NVT, URL de flux avec mot de passe de la NVT. Elles n'ont pas ete captees au vol : **aucun secret, aucune
adresse reelle**. Les marqueurs `{{HOST}}`, `{{HOST_ONLY}}`, `{{PASSWORD}}`, `{{YEAR}}`... sont remplaces par le faux serveur.

| Dossier | Famille | Particularites |
|---|---|---|
| `thingino` | Jooan Q3R / A2R (Thingino stable) | Device, Media, PTZ, Imaging ; chemins `*_service` |
| `vatilon` | Ascam / Vatilon PB1 (firmware constructeur) | chemins `/onvif/Media`, profils H.265 |
| `xiaomi` | Xiaomi sous Thingino master | Device et Media seulement (pas de PTZ) |
| `nvt` | « NVT IPC » (ONVIF 16.12) | port 8899, prefixes `env:`, URL de flux **avec le mot de passe en clair** |

A confronter aux vraies cameras des qu'un `deploy/.env.cameras` est disponible : `cargo test -p dominet-surveillance --test real_onvif -- --ignored`.
