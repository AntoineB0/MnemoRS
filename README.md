# MnemoRS

Émulateur de terminal avec un assistant IA qui connaît l'historique des commandes exécutées.

## Fonctionnalités

- Terminal complet (PTY + xterm.js) : couleurs, `vim`, `htop`, redimensionnement.
- Chaque commande est enregistrée avec son dossier, son code de retour, sa durée et sa sortie.
- Historique consultable et cherchable (SQLite FTS5), filtre échecs.
- Chat avec un modèle local ou distant, qui reçoit en contexte les dernières commandes.
- Indexation de la documentation (`man`, `--help`, READMEs) et recherche hybride.

## Prérequis

- Linux (testé sur Fedora 44 / KDE Plasma)
- Rust (via [rustup](https://rustup.rs))
- Node.js 20 ou plus
- Bibliothèques système :

```bash
sudo dnf install gcc gcc-c++ make pkgconf-pkg-config \
  webkit2gtk4.1-devel gtk3-devel librsvg2-devel \
  libappindicator-gtk3-devel openssl-devel
```

- Un shell `zsh` (l'intégration de capture est fournie pour zsh)
- [Ollama](https://ollama.com) pour un modèle local :

```bash
ollama pull qwen3.5:9b      # modèle de chat
ollama pull bge-m3          # modèle d'embeddings (indexation de la doc)
```

## Installation

```bash
git clone <dépôt> MnemoRS
cd MnemoRS
npm install
```

## Lancement en développement

```bash
npm run tauri dev
```

L'application ouvre une fenêtre avec le terminal à gauche et un panneau à droite
(Historique, Chat, Réglages).

## Compilation

```bash
npm run tauri build
```

Le binaire et les paquets sont produits dans `src-tauri/target/release/bundle`.

## Installation sur le système (Fedora)

```bash
npm run tauri build
sudo dnf install src-tauri/target/release/bundle/rpm/MnemoRS-*.rpm
```

L'application apparaît ensuite dans le menu des applications sous le nom
**MnemoRS** (catégorie Développement). Le paquet installe le binaire dans
`/usr/bin/MnemoRS`, l'icône et l'entrée de bureau.

Désinstallation :

```bash
sudo dnf remove MnemoRS
```

Les données (`~/.local/share/com.icetech.mnemors`) ne sont pas supprimées.

## Vérifications

```bash
cd src-tauri && cargo test     # tests Rust
npm run check                  # types et composants Svelte
```

Certains tests d'intégration nécessitent Ollama et sont ignorés par défaut :

```bash
cd src-tauri
cargo test -- --ignored
```

## Configuration

### Providers

Onglet **Réglages**. Par défaut, un provider Ollama local est créé
(`http://localhost:11434/v1`). Les clés API sont stockées dans le trousseau du
système, jamais dans la base ni dans un fichier.

Exemples :

| Provider | URL de base | Modèle |
|---|---|---|
| Ollama | `http://localhost:11434/v1` | `qwen3.5:9b` |
| OpenRouter | `https://openrouter.ai/api/v1` | `anthropic/claude-3.5-sonnet` |
| Mistral | `https://api.mistral.ai/v1` | `mistral-large-latest` |

### Documentation

Onglet **Réglages**, section *Documentation indexée* :

1. Vérifier l'URL des embeddings (`http://localhost:11434/v1`) et le modèle (`bge-m3`).
2. Cliquer sur **indexer**.

L'indexation parcourt les outils présents dans l'historique (`man`, puis `--help`)
et les fichiers Markdown du dossier courant, en ignorant `node_modules`, `target`,
`.git` et `dist`.

### Corrections

Quand une commande échoue puis qu'une commande réussit dans le même dossier,
le couple « échec → correction » reçoit un score de confiance :

- au-dessus du seuil automatique, il est injecté directement dans le contexte ;
- entre les deux seuils, il attend une validation dans **Réglages → Corrections** ;
- en dessous, il est ignoré.

La fenêtre de temps et les seuils sont réglables. L'historique existant est
analysé une seule fois au premier lancement, en ne conservant que les
certitudes. Une correction erronée peut être retirée depuis le détail de la
commande dans l'historique.

## Données

- Base SQLite : `~/.local/share/com.icetech.mnemors/mnemors.db`
- Intégration shell générée : `~/.local/share/com.icetech.mnemors/shell/zsh/`

La base contient l'historique des commandes. Les commandes sensibles (`sudo`,
`ssh`, `gpg`…) sont enregistrées sans leur sortie, et les secrets évidents
(`token=`, `Bearer …`, clés `sk-…`) sont masqués avant écriture.

## Raccourcis

| Raccourci | Action |
|---|---|
| `Ctrl+Shift+C` | copier la sélection |
| `Ctrl+Shift+V` | coller |
| `Entrée` (chat) | envoyer |
| `Maj+Entrée` (chat) | saut de ligne |

## Architecture

```
src/                  frontend Svelte 5
  lib/Terminal.svelte     xterm.js + PTY
  lib/HistoryPanel.svelte historique et recherche
  lib/ChatPanel.svelte    conversation
  lib/SettingsPanel.svelte providers et indexation
src-tauri/            backend Rust
  src/pty/                pseudo-terminal
  src/capture/            parseur OSC 133
  src/shell/              intégration zsh
  src/storage/            SQLite, migrations, FTS5
  src/providers/          client OpenAI-compatible, SSE
  src/harness/            construction du contexte
  src/docs/               ingestion, découpage, embeddings, recherche
```
