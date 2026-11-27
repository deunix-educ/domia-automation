# ADR 0066 — L'application Android (DomoComm) vit dans le dépôt, APK signé sur GitHub

Statut : **implémenté** (2026-11-26, lots 1 et 2 ; l'APK est publié au lot 3).

## Contexte
Le serveur relaie les SMS et les appels par une application Android ([ADR 0037](0037-android-relay-and-messaging.fr.md), [ADR 0039](0039-android-relay-server.fr.md)). Cette application vivait dans un autre projet privé (`VideoComm`, d'un projet antérieur), n'était pas dans ce dépôt, n'avait pas de licence déclarée, embarquait l'autorité mkcert personnelle de l'auteur et avait deux défauts : la pastille d'un serveur configuré ne devenait jamais verte (le service ne signalait jamais une connexion à l'écran), et le service refusait de démarrer si les huit permissions n'étaient pas toutes accordées.

Qui veut le relais SMS a besoin de l'application. Sans elle dans le dépôt, la fonction documentée de DomiNet n'est pas reproductible.

## Options
- **A. La garder à l'extérieur, documenter le protocole.** Rien à maintenir, mais personne ne peut installer le relais.
- **B. Copier le code dans le dépôt (`android/`) et publier un APK signé comme fichier d'une release GitHub.** Un seul endroit pour le code et pour le serveur qui lui parle ; la compilation reste hors de `make ci`. **Retenue.**
- **C. Publier sur le Play Store ou F-Droid.** Touche plus de monde, mais exige un compte, des revues de règles (les permissions SMS et journal d'appels y sont restreintes) et un engagement durable. Possible plus tard, inutile maintenant.
- **D. Une page web ou une application de passerelle SMS existante.** Autre conception, autre protocole : hors sujet.

## Décision
Option B.
- **Code.** `android/` (Gradle, Kotlin, AGPL-3.0-or-later, commentaires en anglais, textes en anglais et en français). Renommée `DomoComm` (application) et `DomoConf` (package `com.domoconf.comm`, qui est aussi l'identifiant de l'application : pour Android, une nouvelle application, l'ancienne `VideoComm` se désinstalle).
- **Aucune autorité embarquée.** L'autorité mkcert personnelle est retirée. L'application fait confiance aux autorités d'Android et à celles que l'utilisateur a installées sur le téléphone (`network_security_config.xml`). Le clair reste refusé.
- **Sauvegardes désactivées.** Le jeton d'un serveur est dans les préférences privées ; `allowBackup` est faux pour qu'il n'arrive pas dans une sauvegarde.
- **Signature.** Une clé de publication créée hors du dépôt, lue dans `~/.config/domocomm/signing.properties` (ou `DOMOCOMM_SIGNING`). Le dépôt ne fait qu'ignorer les fichiers de clé. L'APK est compilé et contrôlé par `make android-apk` (`tools/build-android.sh` : JDK 17 ou 21, SDK Android, `apksigner verify`), qui écrit `dist/DomoComm-<version>.apk` et affiche son SHA-256 et l'empreinte du certificat. L'empreinte est publiée dans la documentation pour que l'on puisse vérifier ce que l'on installe.
- **Publication.** Une release GitHub à part, étiquette `android-<version>`, indépendante de la version du serveur, avec l'APK et son SHA-256 dans la note. La compilation ne publie rien.
- **Barrière.** `make ci` ne compile pas l'application (cela demanderait le SDK sur chaque machine et à chaque poussée). La compilation et le lint Android sont lancés à la main avant une release.

## Conséquences
- La clé est un secret de longue durée : **sans elle, aucune copie installée ne peut être mise à jour** (Android refuse une autre signature). Elle doit être sauvegardée hors de la machine. Un fork signe avec sa propre clé et est, pour Android, une autre application.
- L'application reste un relais mince : pas encore de tests propres, pas de fiche sur un magasin. Le protocole est couvert côté serveur (`android_ws`, `android_oracle`).
- L'installation depuis un fichier oblige l'utilisateur à autoriser les *paramètres restreints* sous Android 13 et plus (documenté dans [Relais Android](../android.md)) ; c'est une règle de la plateforme, pas un choix.
- La pastille de chaque serveur suit le service par un écouteur posé par l'écran tant qu'il est visible ; un compteur de génération écarte le rapport tardif d'une connexion fermée par un rechargement.

## Tel que construit (2026-11-26)
- Lot 1 (`4395f71`) : import, renommage, pastille, permissions, autorité retirée. Vérifié sur un téléphone (GrapheneOS, Android 15) : la pastille devient verte, le service démarre avec une permission refusée, `wss://` par le VPN fonctionne.
- Lot 2 : signature, `make android-apk`, documentation. Le lint Android passe sans erreur (33 avertissements restants : versions de bibliothèques, icône sans couche monochrome).
- **Non fait** : un test unitaire de l'application, une compilation dans `make ci-full`, un chemin de mise à jour testé d'une version à la suivante.
