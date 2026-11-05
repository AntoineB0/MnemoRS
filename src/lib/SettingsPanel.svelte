<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'
  import { theme, presets, applyTheme, saveCustom, deleteCustom, type Theme } from './theme.svelte'

  type ProviderView = {
    id: string
    name: string
    baseUrl: string
    model: string
    hasKey: boolean
    keyError: string | null
  }

  type DocsSettings = { baseUrl: string; model: string; autoIndex: boolean }
  type DocsStats = { chunks: number; deleted: number; sources: string[]; models: string[] }
  type SyncReport = {
    indexed: number
    skipped: number
    restored: number
    deleted: number
    chunks: number
    failed: number
    cancelled: boolean
  }
  type FixSettings = { windowMinutes: number; autoThreshold: number; suggestThreshold: number }
  type FixRow = {
    id: number
    failedCmdId: number
    fixCmdId: number
    failedCmd: string
    fixCmd: string
    cwd: string | null
    confidence: number
    status: string
    ts: number
  }

  type IngestEvent =
    | { type: 'progress'; done: number; total: number; message: string }
    | { type: 'done'; report: SyncReport }

  let docs = $state<DocsSettings>({ baseUrl: '', model: '', autoIndex: true })
  let stats = $state<DocsStats>({ chunks: 0, deleted: 0, sources: [], models: [] })
  let ingesting = $state(false)
  let autoIndexing = $state(false)
  let progress = $state('')
  let fixSettings = $state<FixSettings>({
    windowMinutes: 10,
    autoThreshold: 0.85,
    suggestThreshold: 0.5,
  })
  let pendingFixes = $state<FixRow[]>([])
  let draft = $state<Theme>({ ...theme.current })

  let providers = $state<ProviderView[]>([])
  let editing = $state<ProviderView | null>(null)
  let apiKey = $state('')
  let message = $state('')
  let error = $state('')
  let busy = $state(false)

  async function load() {
    try {
      providers = await invoke<ProviderView[]>('providers_list')
    } catch (e) {
      error = String(e)
    }
  }

  async function loadDocs() {
    try {
      docs = await invoke<DocsSettings>('docs_settings')
      stats = await invoke<DocsStats>('docs_stats')
    } catch (e) {
      error = String(e)
    }
  }

  async function loadFixes() {
    try {
      fixSettings = await invoke<FixSettings>('fixes_settings')
      pendingFixes = await invoke<FixRow[]>('fixes_pending')
    } catch (e) {
      error = String(e)
    }
  }

  async function saveFixes() {
    try {
      await invoke('fixes_set_settings', {
        windowMinutes: fixSettings.windowMinutes,
        autoThreshold: fixSettings.autoThreshold,
        suggestThreshold: fixSettings.suggestThreshold,
      })
      await loadFixes()
      message = 'Réglages des corrections enregistrés.'
    } catch (e) {
      error = String(e)
    }
  }

  function previewTheme() {
    applyTheme(draft, false)
  }

  function useTheme(preset: Theme) {
    draft = { ...preset }
    applyTheme(draft)
  }

  function saveTheme() {
    saveCustom({ ...draft })
    message = 'Thème enregistré.'
  }

  async function decideFix(fix: FixRow, status: 'confirmed' | 'rejected') {
    try {
      await invoke('fixes_set_status', { id: fix.id, status })
      await loadFixes()
    } catch (e) {
      error = String(e)
    }
  }

  onMount(() => {
    void load()
    void loadDocs()
    void loadFixes()

    const stops: (() => void)[] = []
    listen<boolean>('docs:indexing', (event) => {
      autoIndexing = event.payload
    }).then((stop) => stops.push(stop))
    listen<SyncReport>('docs:indexed', (event) => {
      if (event.payload.indexed > 0 || event.payload.deleted > 0) {
        message = `Indexation automatique : ${event.payload.indexed} source(s) mise(s) à jour.`
      }
      void loadDocs()
    }).then((stop) => stops.push(stop))

    return () => stops.forEach((stop) => stop())
  })

  async function saveDocs() {
    try {
      await invoke('docs_set_settings', {
        baseUrl: docs.baseUrl,
        model: docs.model,
        autoIndex: docs.autoIndex,
      })
      message = "Réglages d'indexation enregistrés."
    } catch (e) {
      error = String(e)
    }
  }

  async function ingest() {
    ingesting = true
    error = ''
    message = ''
    progress = ''
    try {
      await invoke('docs_set_settings', {
        baseUrl: docs.baseUrl,
        model: docs.model,
        autoIndex: docs.autoIndex,
      })
      const channel = new Channel<IngestEvent>()
      channel.onmessage = (event) => {
        if (event.type === 'progress') {
          progress = `${event.done}/${event.total} — ${event.message}`
        } else {
          progress = ''
          const report = event.report
          message = report.cancelled
            ? `Indexation annulée — ${report.indexed} source(s) indexée(s) avant l'arrêt.`
            : `${report.indexed} source(s) indexée(s), ${report.skipped} inchangée(s)` +
              (report.restored ? `, ${report.restored} restaurée(s)` : '') +
              (report.deleted ? `, ${report.deleted} supprimée(s)` : '') +
              (report.failed ? `, ${report.failed} en échec` : '') +
              ` — ${report.chunks} chunks.`
        }
      }
      await invoke('docs_ingest', { onEvent: channel })
      await loadDocs()
    } catch (e) {
      error = String(e)
    } finally {
      ingesting = false
    }
  }

  async function cancelIngest() {
    try {
      await invoke('docs_cancel')
    } catch (e) {
      error = String(e)
    }
  }

  async function clearDocs() {
    try {
      await invoke('docs_clear')
      await loadDocs()
      message = 'Index de documentation vidé.'
    } catch (e) {
      error = String(e)
    }
  }

  function edit(provider?: ProviderView) {
    editing = provider
      ? { ...provider }
      : { id: '', name: '', baseUrl: 'http://localhost:11434/v1', model: '', hasKey: false, keyError: null }
    apiKey = ''
    message = ''
    error = ''
  }

  async function save() {
    if (!editing) return
    busy = true
    error = ''
    try {
      await invoke('providers_save', {
        provider: {
          id: editing.id.trim(),
          name: editing.name.trim() || editing.id.trim(),
          baseUrl: editing.baseUrl.trim(),
          model: editing.model.trim(),
        },
        apiKey: apiKey.trim() || null,
      })
      editing = null
      apiKey = ''
      await load()
      message = 'Provider enregistré.'
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }

  async function remove(provider: ProviderView) {
    try {
      await invoke('providers_delete', { id: provider.id })
      await load()
      message = `${provider.name} supprimé.`
    } catch (e) {
      error = String(e)
    }
  }

  async function removeKey(provider: ProviderView) {
    busy = true
    error = ''
    try {
      await invoke('providers_delete_key', { id: provider.id })
      await load()
      message = `Clé de ${provider.name} supprimée.`
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }

  async function test(provider: ProviderView) {
    busy = true
    error = ''
    message = ''
    try {
      message = await invoke<string>('providers_test', { id: provider.id })
    } catch (e) {
      error = String(e)
    } finally {
      busy = false
    }
  }
</script>

<div class="settings">
  <div class="list">
    {#each providers as provider (provider.id)}
      <div class="item">
        <div class="info">
          <strong>{provider.name}</strong>
          <span class="muted">{provider.model}</span>
          <span class="muted url">{provider.baseUrl}</span>
          <span class="tag" title={provider.keyError ?? ''}>
            {provider.keyError
              ? 'clé inaccessible'
              : provider.hasKey
                ? 'clé enregistrée'
                : 'sans clé'}
          </span>
        </div>
        <div class="actions">
          <button onclick={() => test(provider)} disabled={busy}>tester</button>
          <button onclick={() => edit(provider)}>modifier</button>
          {#if provider.hasKey}
            <button onclick={() => removeKey(provider)} disabled={busy}>supprimer la clé</button>
          {/if}
          <button class="danger" onclick={() => remove(provider)}>supprimer</button>
        </div>
      </div>
    {:else}
      <p class="muted pad">Aucun provider.</p>
    {/each}
  </div>

  {#if editing}
    <form class="form" onsubmit={(e) => { e.preventDefault(); void save() }}>
      <label>
        identifiant
        <input bind:value={editing.id} placeholder="ollama" required />
      </label>
      <label>
        nom affiché
        <input bind:value={editing.name} placeholder="Ollama (local)" />
      </label>
      <label>
        URL de base
        <input bind:value={editing.baseUrl} placeholder="http://localhost:11434/v1" required />
      </label>
      <label>
        modèle
        <input bind:value={editing.model} placeholder="qwen2.5-coder:7b" required />
      </label>
      <label>
        clé API {editing.hasKey ? '(laisser vide pour conserver)' : ''}
        <input bind:value={apiKey} type="password" autocomplete="off" />
      </label>
      <div class="form-actions">
        <button type="submit" disabled={busy}>enregistrer</button>
        <button type="button" onclick={() => (editing = null)}>annuler</button>
      </div>
    </form>
  {:else}
    <button class="add" onclick={() => edit()}>ajouter un provider</button>
  {/if}

  <section class="docs">
    <h3>Documentation indexée</h3>
    <p class="muted">
      {stats.chunks} chunks · {stats.sources.length} sources
      {#if stats.deleted}· {stats.deleted} chunks de documents supprimés{/if}
      {#if stats.models.length}· modèle {stats.models.join(', ')}{/if}
    </p>

    <label>
      URL des embeddings
      <input bind:value={docs.baseUrl} placeholder="http://localhost:11434/v1" />
    </label>
    <label>
      modèle d'embeddings
      <input bind:value={docs.model} placeholder="bge-m3" />
    </label>

    <label class="check">
      <input type="checkbox" bind:checked={docs.autoIndex} />
      indexer automatiquement au démarrage et au changement de dossier
    </label>

    <div class="actions">
      <button onclick={ingest} disabled={ingesting}>
        {ingesting ? 'indexation…' : 'indexer'}
      </button>
      <button onclick={saveDocs} disabled={ingesting}>enregistrer</button>
      {#if ingesting}
        <button onclick={cancelIngest}>annuler</button>
      {/if}
      <button class="danger" onclick={clearDocs} disabled={ingesting}>vider</button>
    </div>

    {#if autoIndexing}<p class="muted">indexation automatique en cours…</p>{/if}
    {#if progress}<p class="muted">{progress}</p>{/if}

    {#if stats.sources.length}
      <ul class="sources">
        {#each stats.sources.slice(0, 30) as source (source)}
          <li title={source}>{source}</li>
        {/each}
      </ul>
    {/if}
  </section>

  <section class="docs">
    <h3>Corrections</h3>
    <p class="muted">
      Un couple échec → correction est repéré quand une commande réussie suit un échec,
      dans le même dossier et dans la fenêtre ci-dessous.
    </p>

    <div class="grid">
      <label>
        fenêtre (minutes)
        <input type="number" min="1" max="1440" bind:value={fixSettings.windowMinutes} />
      </label>
      <label>
        seuil automatique
        <input type="number" min="0" max="1" step="0.05" bind:value={fixSettings.autoThreshold} />
      </label>
      <label>
        seuil de suggestion
        <input type="number" min="0" max="1" step="0.05" bind:value={fixSettings.suggestThreshold} />
      </label>
    </div>

    <div class="actions">
      <button onclick={saveFixes}>enregistrer</button>
    </div>

    {#if pendingFixes.length}
      <p class="muted">{pendingFixes.length} correction(s) à valider</p>
      <ul class="fixes">
        {#each pendingFixes as fix (fix.id)}
          <li>
            <code>$ {fix.failedCmd}</code>
            <span class="arrow">→</span>
            <code>$ {fix.fixCmd}</code>
            <span class="score">{Math.round(fix.confidence * 100)} %</span>
            <span class="actions">
              <button onclick={() => decideFix(fix, 'confirmed')}>✓</button>
              <button class="danger" onclick={() => decideFix(fix, 'rejected')}>✗</button>
            </span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">Aucune correction en attente.</p>
    {/if}
  </section>

  <section class="docs">
    <h3>Thème</h3>

    <div class="presets">
      {#each presets as preset (preset.id)}
        <button onclick={() => useTheme(preset)}>{preset.name}</button>
      {/each}
    </div>

    <div class="grid colors">
      <label>fond<input type="color" bind:value={draft.void} oninput={previewTheme} /></label>
      <label>panneau<input type="color" bind:value={draft.surface} oninput={previewTheme} /></label>
      <label>survol<input type="color" bind:value={draft.surface2} oninput={previewTheme} /></label>
      <label>bordures<input type="color" bind:value={draft.line} oninput={previewTheme} /></label>
      <label>texte<input type="color" bind:value={draft.ink} oninput={previewTheme} /></label>
      <label>secondaire<input type="color" bind:value={draft.inkDim} oninput={previewTheme} /></label>
      <label>atténué<input type="color" bind:value={draft.inkFaint} oninput={previewTheme} /></label>
      <label>accent<input type="color" bind:value={draft.signal} oninput={previewTheme} /></label>
      <label>erreur<input type="color" bind:value={draft.alert} oninput={previewTheme} /></label>
    </div>

    <label>
      terminal
      <select bind:value={draft.mode} onchange={previewTheme}>
        <option value="dark">sombre</option>
        <option value="light">clair</option>
      </select>
    </label>

    <div class="theme-save">
      <input bind:value={draft.name} placeholder="nom du thème" />
      <button onclick={saveTheme}>enregistrer</button>
    </div>

    {#if theme.custom.length}
      <ul class="fixes">
        {#each theme.custom as item (item.id)}
          <li>
            <button onclick={() => useTheme(item)}>{item.name}</button>
            <button class="danger" onclick={() => deleteCustom(item.id)}>supprimer</button>
          </li>
        {/each}
      </ul>
    {/if}
  </section>

  {#if message}<p class="ok">{message}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
  <p class="hint">
    Les clés sont stockées dans le trousseau du système, jamais dans la base ni dans un fichier.
  </p>
</div>

<style>
  .settings {
    padding: 12px;
    overflow-y: auto;
    height: 100%;
    min-height: 0;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--ink);
  }

  .item {
    padding: 10px 0;
    border-bottom: 1px solid var(--line);
  }

  .item:first-child {
    padding-top: 0;
  }

  .info {
    display: flex;
    flex-direction: column;
    gap: 3px;
    margin-bottom: 6px;
  }

  .url {
    color: var(--ink-dim);
    font-size: 11px;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  button {
    padding: 4px 8px;
    background: transparent;
    border: 1px solid var(--line);
    color: var(--ink-dim);
    font-family: var(--mono);
    font-size: 11px;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    border-color: var(--signal);
    color: var(--signal);
  }

  button:disabled {
    color: var(--ink-faint);
    border-color: var(--line);
    cursor: default;
  }

  .danger {
    border-color: var(--alert);
    color: var(--alert);
  }

  .danger:hover:not(:disabled) {
    border-color: var(--alert);
    color: var(--alert);
  }

  .add {
    width: 100%;
    margin-top: 10px;
    padding: 6px;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 10px 0;
    border-top: 1px solid var(--line);
    margin-bottom: 10px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    color: var(--ink-dim);
    font-size: 11px;
  }

  input {
    padding: 6px 8px;
    background: var(--void);
    border: 1px solid var(--line);
    color: var(--ink);
    font-family: var(--mono);
    font-size: 12px;
  }

  input:focus {
    outline: 1px solid var(--signal);
    outline-offset: 0;
  }

  .form-actions {
    display: flex;
    gap: 6px;
  }

  .tag {
    align-self: flex-start;
    border: 1px solid var(--line);
    padding: 0 4px;
    color: var(--ink-dim);
    font-size: 10px;
  }

  .muted {
    color: var(--ink-dim);
  }

  .pad {
    padding: 8px 0;
  }

  .ok {
    color: var(--signal);
  }

  .error {
    color: var(--alert);
  }

  .hint {
    color: var(--ink-faint);
    font-size: 11px;
    line-height: 1.5;
  }

  .docs {
    margin-top: 18px;
    border-top: 1px solid var(--line);
    padding-top: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .docs h3 {
    margin: 0;
    font-family: var(--mono);
    font-size: 11px;
    font-weight: 400;
    color: var(--ink-dim);
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .presets {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }

  input[type='color'] {
    height: 28px;
    padding: 2px;
    background: var(--void);
    border: 1px solid var(--line);
    cursor: pointer;
  }

  .theme-save {
    display: flex;
    gap: 6px;
  }

  .theme-save input {
    flex: 1;
    min-width: 0;
  }

  select {
    padding: 6px 8px;
    background: var(--void);
    border: 1px solid var(--line);
    color: var(--ink);
    font-family: var(--mono);
    font-size: 12px;
    appearance: none;
    -webkit-appearance: none;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    color: var(--ink-dim);
    font-size: 11px;
  }

  .check input {
    accent-color: var(--signal);
  }

  .fixes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
  }

  .fixes li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 8px 0;
    border-bottom: 1px solid var(--line);
    font-size: 11px;
  }

  .fixes code {
    font-family: var(--mono);
    color: var(--ink);
    overflow-wrap: anywhere;
  }

  .arrow {
    color: var(--ink-dim);
  }

  .score {
    color: var(--ink-dim);
    margin-left: auto;
  }

  .fixes .actions {
    display: flex;
    gap: 4px;
  }

  .sources {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 140px;
    overflow-y: auto;
    font-size: 11px;
    color: var(--ink-dim);
  }

  .sources li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
