<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'

  type ProviderView = {
    id: string
    name: string
    baseUrl: string
    model: string
    hasKey: boolean
  }

  type DocsSettings = { baseUrl: string; model: string; autoIndex: boolean }
  type DocsStats = { chunks: number; deleted: number; sources: string[]; models: string[] }
  type SyncReport = {
    indexed: number
    skipped: number
    restored: number
    deleted: number
    chunks: number
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
          message =
            `${report.indexed} source(s) indexée(s), ${report.skipped} inchangée(s)` +
            (report.restored ? `, ${report.restored} restaurée(s)` : '') +
            (report.deleted ? `, ${report.deleted} supprimée(s)` : '') +
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
      : { id: '', name: '', baseUrl: 'http://localhost:11434/v1', model: '', hasKey: false }
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
          <span class="tag">{provider.hasKey ? 'clé enregistrée' : 'sans clé'}</span>
        </div>
        <div class="actions">
          <button onclick={() => test(provider)} disabled={busy}>tester</button>
          <button onclick={() => edit(provider)}>modifier</button>
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

  {#if message}<p class="ok">{message}</p>{/if}
  {#if error}<p class="error">{error}</p>{/if}
  <p class="hint">
    Les clés sont stockées dans le trousseau du système, jamais dans la base ni dans un fichier.
  </p>
</div>

<style>
  .settings {
    padding: 10px 12px 16px;
    overflow-y: auto;
    height: 100%;
    font-size: 12px;
  }

  .item {
    border: 1px solid #2a2b38;
    border-radius: 5px;
    padding: 8px;
    margin-bottom: 8px;
  }

  .info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-bottom: 6px;
  }

  .url {
    font-family: ui-monospace, monospace;
    overflow-wrap: anywhere;
  }

  .actions {
    display: flex;
    gap: 6px;
  }

  button {
    font: inherit;
    color: inherit;
    background: #242535;
    border: 1px solid #34354a;
    border-radius: 4px;
    padding: 4px 8px;
    cursor: pointer;
  }

  button:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .danger {
    border-color: #5b3030;
    color: #f4a2a2;
  }

  .add {
    width: 100%;
    margin-top: 4px;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid #2a2b38;
    border-radius: 5px;
    padding: 10px;
    margin-bottom: 10px;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 3px;
    color: #9b9bab;
  }

  input {
    font: inherit;
    color: inherit;
    background: #0f1016;
    border: 1px solid #2a2b38;
    border-radius: 4px;
    padding: 5px 7px;
  }

  input:focus {
    outline: 1px solid #c084fc;
  }

  .form-actions {
    display: flex;
    gap: 6px;
  }

  .tag {
    align-self: flex-start;
    border: 1px solid #34354a;
    border-radius: 3px;
    padding: 0 4px;
    color: #9b9bab;
    font-size: 10px;
  }

  .muted {
    color: #82839a;
  }

  .pad {
    padding: 8px 0;
  }

  .ok {
    color: #8bd49c;
  }

  .error {
    color: #f48771;
  }

  .docs {
    margin-top: 16px;
    border-top: 1px solid #2a2b38;
    padding-top: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .docs h3 {
    margin: 0;
    font: 600 11px/1 system-ui, sans-serif;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: #9b9bab;
  }

  .check {
    flex-direction: row;
    align-items: center;
    gap: 6px;
    color: #9b9bab;
    font-size: 11px;
  }

  .docs .actions {
    flex-wrap: wrap;
  }

  .grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
  }

  .fixes {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .fixes li {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    border: 1px solid #2a2b38;
    border-radius: 4px;
    padding: 6px;
    font-size: 11px;
  }

  .fixes code {
    font-family: ui-monospace, monospace;
    overflow-wrap: anywhere;
  }

  .arrow {
    color: #82839a;
  }

  .score {
    color: #9b9bab;
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
    font-family: ui-monospace, monospace;
    font-size: 11px;
    color: #82839a;
  }

  .sources li {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hint {
    color: #6b6b7f;
    font-size: 11px;
    line-height: 1.4;
  }
</style>
