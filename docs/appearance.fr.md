# Apparence : personnaliser les couleurs, la police, les largeurs

Ce qui se règle **sans toucher au code** (menu, tableaux de bord, widgets) se fait dans l'administration : voir [Appareils](devices.fr.md) et l'[interface](interface.fr.md). Cette page décrit le **style** : couleurs, police, arrondis, largeur du menu.

## Le fichier `custom.css`
DomiNet charge, après sa feuille de style principale, le fichier `deploy/custom/custom.css` s'il existe (variable `CUSTOM_CSS`, montage `./custom:/custom:ro` du compose de production). Il est **relu à chaque chargement de page** : une modification se voit en rechargeant le navigateur, **sans redémarrage ni reconstruction de l'image**. Il est ignoré par git : c'est un réglage de la machine.

```bash
cd ~/opt/dominet/deploy/custom
cp custom.css.example custom.css
$EDITOR custom.css            # décommentez et modifiez les variables voulues
```
Rechargez la page (Ctrl+Maj+R si le navigateur garde l'ancienne version). **Retour à l'apparence d'origine** : supprimez `custom.css`. Un fichier absent, illisible ou de plus de 256 Kio donne une feuille vide : jamais une erreur.

## Les variables
Elles se placent dans un bloc `:root { … }`.

| Variable | Rôle | Défaut |
|---|---|---|
| `--bg-base` | fond de page | `#0d1117` |
| `--bg-surface` | fond du menu et des cartes | `#1c2128` |
| `--bg-elevated`, `--bg-hover` | éléments en relief, survol | `#2d333b`, `#373e47` |
| `--accent`, `--accent-dim`, `--accent-glow` | couleur principale, sa variante sombre, son halo | `#00d4aa`, `#00a884`, vert translucide |
| `--danger`, `--warning`, `--info`, `--success` | couleurs d'état (erreur, avertissement, information, succès) | `#f85149`, `#e3b341`, `#58a6ff`, `#56d364` |
| `--text-primary`, `--text-secondary`, `--text-muted` | niveaux de texte | `#f0f6fc`, `#b1bac4`, `#768390` |
| `--border` | filets et bordures | `#30363d` |
| `--radius-sm`, `--radius-md`, `--radius-lg` | arrondis | `6px`, `10px`, `16px` |
| `--sidebar-w` | largeur du menu de gauche | `220px` |
| `--font` | police | `'Segoe UI', system-ui, …` |

Exemple (thème orange, menu plus large) :
```css
:root {
    --accent: #ff9f1a;
    --accent-dim: #d98200;
    --sidebar-w: 260px;
}
```
Au-delà des variables, vous pouvez écrire n'importe quelle règle CSS (par exemple `.vc-table td { font-size: .85rem; }`) ; les classes sont celles de `crates/dominet-web/style/main.css`.

## À savoir
- Le thème reste **sombre** : il n'y a pas de variante claire. Une couleur de fond claire demande aussi de changer les couleurs de texte et de filet.
- La feuille est servie par DomiNet sur `/custom.css`, publique (la page de connexion est habillée aussi). Elle ne doit contenir aucun secret.
- Le fichier doit se trouver dans `deploy/custom/` (monté en lecture seule). Une modification faite ailleurs n'est pas vue.
- **Développement** : sans la variable `CUSTOM_CSS`, `/custom.css` répond une feuille vide.
