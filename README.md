<p align="center"><img src="assets/logo.svg" width="112" alt="Logo de Lyra"></p>

# OpenLyra

**Lyra** est un logiciel de bureau écrit en Rust pour **analyser** puis **supprimer** les métadonnées cachées d'un document. Tout se passe sur ton ordinateur : aucun document n'est envoyé sur Internet.

![Analyse d'une photo](docs/capture_analyse.png)

## Formats et métadonnées pris en charge

| Format | Ce qui est détecté et supprimable |
|---|---|
| **JPEG** | EXIF détaillé (GPS, appareil, numéros de série, auteur, dates, réglages, miniature), XMP, IPTC/Photoshop, commentaires, blocs propriétaires, données cachées après l'image, profil ICC |
| **PNG** | Textes intégrés (auteur, logiciel, commentaires, paramètres de génération d'images IA…), EXIF, XMP, date de modification, blocs non standard, profil ICC |
| **PDF** | Auteur, titre/sujet/mots-clés, logiciels, dates, propriétés personnalisées, XMP du document et des images, annotations et leurs auteurs, identifiant du fichier. Les versions antérieures cachées (mises à jour incrémentales) sont toujours supprimées. |
| **Word, Excel, PowerPoint** (docx, xlsx, pptx…) | Auteurs, société, responsable, dates, titre, modèle (et son chemin), temps d'édition, nombre de révisions, propriétés personnalisées, miniature, imprimante, métadonnées des photos insérées. Pour Word : commentaires et suivi des modifications (accepté automatiquement). |
| **LibreOffice** (odt, ods, odp…) | Créateur, dernier auteur, imprimé par, dates, modèle, durée d'édition, propriétés personnalisées, miniature, imprimante, photos insérées, commentaires, suivi des modifications (texte). |

### Traces dans le contenu (Word et LibreOffice)

Au-delà des propriétés, Lyra détecte et retire les traces cachées dans le contenu même du document :

| Catégorie | Ce que c'est | Par défaut |
|---|---|---|
| Chemins de fichiers locaux | Liens, images liées, publipostage pointant vers `C:\Users\…` (révèle souvent le nom d'utilisateur) | coché |
| Champs liés à l'auteur | Champs AUTEUR, NOM D'UTILISATEUR, INITIALES, données de l'expéditeur, chemin du fichier | coché |
| Identifiants de session d'édition | Codes (rsid) qui relient le document à d'autres édités sur le même PC ou aux mêmes moments | coché |
| Variables cachées | Valeurs stockées par des macros, modèles ou logiciels | coché |
| Paramètres enregistrés (LibreOffice) | Source de données de publipostage, télécopieur | coché |
| Texte masqué | Texte présent dans le fichier mais invisible | à cocher |
| Signets | Noms internes renommés de façon neutre, `_GoBack` supprimé ; table des matières et renvois continuent de fonctionner | à cocher |

Le texte visible n'est jamais réécrit : Lyra ne modifie ni le style ni le fond du document.

Le nettoyage EXIF est **sélectif** : on peut par exemple retirer le GPS et l'appareil tout en gardant l'orientation, pour que la photo ne pivote pas.

## Installation

### Télécharger Lyra (recommandé)

Va dans la section [**Releases**](https://github.com/Powerl3ss/OpenLyra/releases/latest) :

- **Windows** : télécharge `Lyra-windows.exe` et lance-le. Aucun installateur. Si Windows affiche « Windows a protégé votre ordinateur » (l'exécutable n'est pas signé), clique sur *Informations complémentaires* puis *Exécuter quand même*.
- **macOS** (puces Apple et Intel, macOS 11 ou plus récent) : télécharge `Lyra-macos.dmg`, ouvre-le et glisse **Lyra** dans **Applications**. Au premier lancement, macOS bloque l'application car elle n'est pas signée par un développeur Apple : ouvre *Réglages Système > Confidentialité et sécurité*, puis clique sur *Ouvrir quand même* en face de Lyra. Sur les versions plus anciennes de macOS, un clic droit sur Lyra puis *Ouvrir* suffit.
- **Linux** : télécharge `Lyra-linux`, rends-le exécutable (`chmod +x Lyra-linux`) puis lance-le.

### Mises à jour automatiques

Au démarrage, Lyra vérifie s'il existe une version plus récente sur GitHub. Si c'est le cas, un bandeau propose de **mettre à jour** : la nouvelle version est téléchargée, son empreinte SHA-256 est vérifiée, puis l'exécutable est remplacé. Il suffit ensuite de cliquer sur **Redémarrer Lyra**. Le bouton *Mises à jour* en haut à droite permet aussi de vérifier à la main.

Lyra ne contacte Internet que pour cette vérification (API publique de GitHub). Tes documents ne quittent jamais ton ordinateur.

## Limites connues

- Les PDF chiffrés ou protégés par mot de passe sont refusés (retire la protection avant).
- Les fichiers joints à l'intérieur d'un PDF ne sont pas nettoyés (un avertissement s'affiche).
- Pour Excel et PowerPoint, les commentaires sont signalés mais doivent être supprimés depuis l'application.
- Les métadonnées ne sont pas tout : le **contenu visible** (texte, en-têtes, noms dans le corps du document, visages sur une photo) n'est pas modifié.

## Structure du code

```
src/
├── main.rs          point d'entrée (interface ou ligne de commande)
├── gui.rs           interface egui
├── model.rs         rapport, catégories, niveaux de risque
├── xml.rs           lecture et réécriture XML en flux
├── exifmeta.rs      analyse et reconstruction sélective de l'EXIF
├── xmp.rs           lecture des paquets XMP
├── updater.rs       mises à jour depuis les releases GitHub
├── splash.rs        écran de chargement (œil ASCII animé)
└── formats/
    ├── mod.rs       détection du format et aiguillage
    ├── jpeg.rs  png.rs  pdf.rs
    ├── zipdoc.rs    conteneur zip commun (Office / LibreOffice)
    ├── ooxml.rs     Word, Excel, PowerPoint
    ├── traces.rs    traces dans le contenu (Word, LibreOffice)
    └── odf.rs       LibreOffice
```

Le logo est dans `assets/` : `logo.svg` (fond sombre), `logo-clair.svg`, `logo-seul.svg` (sans fond), `lyra.ico` (icône de l'exe, intégrée par `build.rs`) et `icon-256.png` (icône de la fenêtre).

Bibliothèques principales : `eframe`/`egui` (interface), `rfd` (boîtes de dialogue natives), `lopdf` (PDF), `kamadak-exif` (EXIF), `quick-xml`, `zip`.
