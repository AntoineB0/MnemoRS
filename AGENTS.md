# Règles de contribution

- Committer par petites unités logiques : une fonctionnalité, un correctif ou une étape du plan par commit.
- Ne jamais ajouter de trailer `Co-authored-by`.
- Commenter uniquement le comportement d'ensemble ou un choix technique. Pas de commentaire qui paraphrase une ligne de code.
- Ne pas passer à l'étape suivante du plan sans avoir vérifié ses critères d'acceptation.
- Avant de committer : `cargo test` dans `src-tauri/` et `npm run check` à la racine.
