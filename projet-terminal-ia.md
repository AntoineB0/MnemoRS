# MnemoRS : émulateur de terminal + harness IA

Document technique : vision, stack, architecture et plan de développement.

---

## 1. Vision

Un émulateur de terminal classique, auquel s'ajoute un harness IA qui connaît tout ce qui s'est passé dans le terminal.

Objectifs :

- **Contexte automatique** : l'IA a accès à l'historique des commandes exécutées (commande, dossier, code de retour, sortie).
- **RAG sur la documentation** : man pages, `--help`, READMEs, fichiers Markdown, indexés localement.
- **Base de connaissances "ce qui marche"** : résultats des commandes passées, avec liaison entre une commande en échec et la commande qui l'a corrigée.
- **Multi-provider** : connexion à plusieurs fournisseurs via le protocole OpenAI (Claude, Mistral, Ollama, OpenRouter, etc.), avec changement à la volée.
- **Local d'abord** : stockage et embeddings en local.

---

## 2. Stack technique

| Couche | Choix |
|---|---|
| Application desktop | Tauri (backend Rust) |
| Affichage du terminal | xterm.js |
| Pseudo-terminal (PTY) | `portable-pty` (Rust) |
| Base de données | SQLite (un seul fichier) |
| Recherche textuelle | SQLite FTS5 |
| Recherche vectorielle | `sqlite-vec` (alternative : LanceDB) |
| Accès SQLite en Rust | `rusqlite` (ou `sqlx` si besoin d'async) |
| Embeddings | Locaux : `fastembed-rs` (alternative : Ollama) |
| API IA | Protocole OpenAI (`/v1/chat/completions`, `/v1/embeddings`) |
| Client HTTP | `reqwest` (streaming SSE) |
| Stockage des clés API | Trousseau de l'OS via la crate `keyring` |

---

## 3. Architecture

```
┌────────────────────────── Tauri ──────────────────────────┐
│  Frontend (WebView)                                       │
│  ├─ xterm.js          terminal                            │
│  ├─ Panneau chat      conversation avec l'IA              │
│  ├─ Panneau historique  recherche, filtres                │
│  └─ Réglages          providers, modèles                  │
│                         ▲ événements / commandes Tauri    │
│  Backend (Rust)         ▼                                 │
│  ├─ Module PTY        shell + hooks de commandes          │
│  ├─ Module stockage   SQLite (historique, doc, vecteurs)  │
│  ├─ Module embeddings fastembed-rs                        │
│  ├─ Module providers  client OpenAI-compatible            │
│  └─ Module harness    construction du contexte, boucle IA │
└───────────────────────────────────────────────────────────┘
```

### Flux principal

1. L'utilisateur tape une commande dans xterm.js.
2. Le backend l'envoie au PTY et capte la sortie, qui revient à l'affichage.
3. Les hooks de shell délimitent la commande : à la fin, un enregistrement est écrit en base.
4. Quand l'utilisateur interroge l'IA, le harness assemble un contexte (historique récent, doc pertinente, cas passés similaires) et appelle le provider actif.
5. La réponse est diffusée en streaming dans le panneau de chat.

---

## 4. Stockage

### 4.1 Historique des commandes

```sql
CREATE TABLE commands (
  id INTEGER PRIMARY KEY,
  cmd TEXT NOT NULL,
  cwd TEXT,
  exit_code INTEGER,
  output TEXT,            -- tronquée
  duration_ms INTEGER,
  ts INTEGER NOT NULL
);

CREATE VIRTUAL TABLE commands_fts USING fts5(
  cmd, output, content='commands', content_rowid='id'
);
```

Pour lier une commande en échec à sa correction :

```sql
CREATE TABLE fixes (
  id INTEGER PRIMARY KEY,
  failed_cmd_id INTEGER REFERENCES commands(id),
  fix_cmd_id INTEGER REFERENCES commands(id)
);
```

### 4.2 Documentation (RAG)

```sql
CREATE TABLE doc_chunks (
  id INTEGER PRIMARY KEY,
  source TEXT,            -- ex : man page, README
  tool TEXT,              -- ex : docker, git (pour filtrer)
  section TEXT,
  content TEXT,
  embed_model TEXT        -- modèle utilisé pour l'embedding
);
-- + table vectorielle sqlite-vec liée à doc_chunks
--   (la dimension des vecteurs est fixée à la création)
```

### 4.3 Recherche hybride

- **FTS5** pour les identifiants exacts (flags, noms de commandes, messages d'erreur).
- **Vecteurs** pour les questions en langage naturel.
- Les deux résultats sont fusionnés avant injection dans le contexte.

---

## 5. Embeddings

- **Local** (`fastembed-rs`) : modèle téléchargé au premier lancement, fonctionnement hors ligne ensuite.
- **Modèle** : multilingue léger au départ (type `multilingual-e5-small`), car la doc est surtout en anglais et les requêtes en français. Option plus lourde : `bge-m3` (plutôt via Ollama).
- **Abstraction** : un `trait Embedder` en Rust, pour pouvoir passer plus tard à Ollama.
- **Cohérence** : les vecteurs de modèles différents sont incompatibles. Le nom du modèle est stocké avec chaque chunk, et un changement de modèle déclenche une réindexation.
- **Séparation** : le provider d'embeddings est indépendant du provider de chat (tous les providers n'ont pas d'embeddings, Claude notamment).

---

## 6. Providers IA

Chaque provider est une configuration :

```
{ nom, base_url, clé_api, modèle }
```

Exemples : Ollama (`http://localhost:11434/v1`), OpenRouter, Mistral, Claude.

Points d'attention :

- **Claude** : la couche de compatibilité OpenAI est surtout destinée aux tests ; prévoir à terme un adaptateur natif.
- **Tool calling** : varie selon les providers, et les petits modèles locaux le gèrent mal. Prévoir un mode de secours où le modèle propose une commande dans un bloc de code, validée par l'utilisateur.
- **Streaming (SSE)** : à gérer dès le début.
- **Clés API** : jamais en clair dans un fichier de config, uniquement via le trousseau de l'OS.

---

## 7. Harness IA

### 7.1 Construction du contexte

Éléments injectés, par ordre de priorité :

1. Les N dernières commandes (avec sorties tronquées)
2. Le dossier courant et les erreurs récentes
3. Les passages de doc pertinents (RAG)
4. Les cas passés similaires (commande en échec puis correction)

Contraintes : budget de tokens limité, troncature des longues sorties, résumé si nécessaire.

### 7.2 Sécurité

- Aucune commande générée par l'IA ne s'exécute sans validation de l'utilisateur au départ.
- Liste de commandes bloquées ou à confirmation renforcée (`rm -rf`, `dd`, `mkfs`, etc.).
- Ne pas enregistrer les saisies de mot de passe ; masquer les secrets évidents (tokens, clés API) dans les sorties stockées.
- Avertir que le contenu du terminal peut être envoyé à un provider distant ; l'utilisateur doit pouvoir choisir un provider local (Ollama) pour les sessions sensibles.

---

## 8. Plan de développement

Chaque étape produit quelque chose d'utilisable seul.

### Partie 1 : le terminal (sans IA)

**Étape 1 : squelette Tauri + xterm.js**
- Créer le projet Tauri et afficher xterm.js.
- Lancer un shell avec `portable-pty`, relier entrée/sortie via les événements Tauri.
- Gérer le redimensionnement.
- *Terminé quand* `ls`, `cd` et `vim` fonctionnent normalement.

**Étape 2 : enregistrement des commandes**
- Détecter début et fin de commande via des hooks de shell (`preexec`/`precmd` en zsh, `PROMPT_COMMAND` en bash) ou les marqueurs OSC 133.
- Stocker dans SQLite : commande, dossier, code de retour, sortie tronquée, durée, date.
- Ne pas enregistrer les mots de passe, masquer les secrets.
- *Terminé quand* chaque commande tapée apparaît en base avec son résultat.
- *Attention* : étape la plus délicate, prévoir du temps.

**Étape 3 : consulter l'historique**
- Panneau latéral avec liste des commandes, recherche FTS5, filtre succès/échec.
- *Terminé quand* retrouver une ancienne commande prend quelques secondes.

### Partie 2 : l'IA (MVP)

**Étape 4 : connexion aux providers**
- Client OpenAI-compatible avec config par provider et streaming.
- Clés dans le trousseau de l'OS.
- Écran de réglages pour ajouter et changer de provider.
- *Terminé quand* un message est envoyé à Ollama puis à OpenRouter avec réponse en direct.

**Étape 5 : harness v1 (contexte = historique)**
- Panneau de chat à côté du terminal.
- Contexte : dernières commandes, dossier courant, erreurs récentes, sorties tronquées.
- L'IA répond uniquement, sans exécuter.
- *Terminé quand* la question « pourquoi ma dernière commande a échoué ? » obtient une réponse fondée sur la vraie sortie.

> Les étapes 1 à 5 constituent le **MVP** (environ 3 semaines).

### Partie 3 : la mémoire

**Étape 6 : RAG sur la doc**
- Ingestion des man pages, `--help` et fichiers Markdown, découpés par sections.
- Embeddings locaux et stockage `sqlite-vec`.
- Recherche hybride FTS5 + vecteurs, injection dans le contexte.
- *Terminé quand* l'IA cite la bonne option d'une commande qu'elle connaît mal.

**Étape 7 : base de connaissances "ce qui marche"**
- Lier une commande en échec à la commande qui l'a corrigée.
- Retrouver les cas similaires et les injecter dans le contexte.
- *Terminé quand* l'IA propose une correction déjà utilisée par le passé.

### Partie 4 : l'agent (facultatif)

**Étape 8 : exécution de commandes par l'IA**
- Le modèle propose une commande, l'utilisateur valide, elle s'exécute dans le terminal, le résultat revient dans la boucle.
- Tool calling avec mode de secours en bloc de code.
- Liste de commandes bloquées, confirmation systématique.
- *Terminé quand* une tâche multi-étapes peut être suivie et validée pas à pas.

---

## 9. Conseils de conduite de projet

- Ne passer à l'étape suivante que si la précédente fonctionne vraiment.
- Utiliser l'outil au quotidien dès l'étape 3 pour trouver bugs et idées.
- S'arrêter à l'étape 7 donne déjà un projet complet et original ; l'étape 8 est un bonus.

---

## 10. Pistes d'évolution

- Adaptateur natif pour Claude (et autres APIs non compatibles).
- Résumé automatique des longues sessions pour économiser le contexte.
- Indexation automatique de la doc des projets présents dans le dossier courant.
- Export et import de la base de connaissances.
- Mode « session sensible » : provider local uniquement, aucun envoi distant.
