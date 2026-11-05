<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { listen } from '@tauri-apps/api/event'

  type CommandRow = {
    id: number
    cmd: string
    cwd: string | null
    exitCode: number | null
    outputTruncated: boolean
    sensitive: boolean
    durationMs: number | null
    ts: number
  }

  type CommandDetail = CommandRow & { output: string }

  type FixRow = {
    id: number
    fixCmd: string
    status: string
    confidence: number
  }

  let query = $state('')
  let onlyFailed = $state(false)
  let rows = $state<CommandRow[]>([])
  let selected = $state<CommandDetail | null>(null)
  let fix = $state<FixRow | null>(null)
  let loading = $state(false)
  let error = $state('')

  async function load(search = query, failed = onlyFailed) {
    try {
      loading = true
      rows = await invoke<CommandRow[]>('history_search', {
        query: search,
        limit: 200,
        onlyFailed: failed,
      })
      error = ''
    } catch (e) {
      error = String(e)
    } finally {
      loading = false
    }
  }

  let timer: number | undefined
  $effect(() => {
    const search = query
    const failed = onlyFailed
    clearTimeout(timer)
    timer = window.setTimeout(() => void load(search, failed), 200)
    return () => clearTimeout(timer)
  })

  onMount(() => {
    let unlisten: (() => void) | undefined
    listen('history:new', () => void load()).then((stop) => (unlisten = stop))
    return () => unlisten?.()
  })

  async function toggle(row: CommandRow) {
    if (selected?.id === row.id) {
      selected = null
      fix = null
      return
    }
    try {
      selected = await invoke<CommandDetail | null>('history_get', { id: row.id })
      fix =
        row.exitCode !== 0
          ? await invoke<FixRow | null>('fixes_for', { cmdId: row.id })
          : null
    } catch (e) {
      error = String(e)
    }
  }

  async function removeFix() {
    if (!fix) return
    try {
      await invoke('fixes_delete', { id: fix.id })
      fix = null
    } catch (e) {
      error = String(e)
    }
  }

  function time(ts: number) {
    return new Date(ts).toLocaleTimeString('fr-FR', {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    })
  }

  function duration(ms: number | null) {
    if (ms === null) return ''
    if (ms < 1000) return `${ms} ms`
    return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text)
    } catch {
      error = 'presse-papiers indisponible'
    }
  }
</script>

<aside class="panel">
  <div class="filters">
    <input
      class="search"
      type="search"
      placeholder="Rechercher commande ou sortie…"
      bind:value={query}
    />
    <label>
      <input type="checkbox" bind:checked={onlyFailed} />
      échecs
    </label>
  </div>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <ul>
    {#each rows as row (row.id)}
      <li class:failed={row.exitCode !== 0}>
        <button class="row" onclick={() => toggle(row)}>
          <span class="meta">
            <span class="code">{row.exitCode ?? '?'}</span>
            <span>{time(row.ts)}</span>
            {#if row.durationMs !== null}<span>{duration(row.durationMs)}</span>{/if}
            {#if row.sensitive}<span class="tag">sensible</span>{/if}
          </span>
          <span class="cmd">{row.cmd}</span>
        </button>

        {#if selected?.id === row.id}
          <div class="detail">
            {#if row.cwd}<p class="cwd" title={row.cwd}>{row.cwd}</p>{/if}
            {#if row.sensitive}
              <p class="muted">Sortie non enregistrée (commande sensible).</p>
            {:else if selected.output}
              <pre>{selected.output}</pre>
              {#if row.outputTruncated}<p class="muted">sortie tronquée</p>{/if}
            {:else}
              <p class="muted">Aucune sortie.</p>
            {/if}
            {#if fix}
              <div class="fix">
                <p class="muted">
                  correction associée
                  {fix.status === 'suggested' ? '(à valider dans les réglages)' : ''}
                </p>
                <pre>$ {fix.fixCmd}</pre>
                <button class="copy" onclick={removeFix}>retirer cette correction</button>
              </div>
            {/if}

            <button class="copy" onclick={() => copy(row.cmd)}>copier la commande</button>
          </div>
        {/if}
      </li>
    {:else}
      <li class="empty">{loading ? 'Chargement…' : 'Aucune commande.'}</li>
    {/each}
  </ul>
</aside>

<style>
  .panel {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-family: var(--mono);
    font-size: 12px;
    color: var(--ink);
  }

  .filters {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }

  label {
    display: flex;
    gap: 5px;
    align-items: center;
    color: var(--ink-dim);
    font-size: 11px;
    white-space: nowrap;
    cursor: pointer;
  }

  input[type='checkbox'] {
    accent-color: var(--signal);
  }

  .search {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    background: var(--void);
    border: 1px solid var(--line);
    color: var(--ink);
    font: inherit;
  }

  .search::placeholder {
    color: #5f7a71;
  }

  .search:focus {
    outline: 1px solid var(--signal);
    outline-offset: 0;
  }

  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 8px;
    overflow-y: auto;
    flex: 1;
  }

  li {
    border-bottom: 1px solid var(--line);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    width: 100%;
    padding: 9px 12px 9px 14px;
    background: none;
    border: 0;
    border-left: 2px solid transparent;
    color: inherit;
    text-align: left;
    cursor: pointer;
    font: inherit;
  }

  .row:hover {
    background: var(--surface-2);
  }

  li.failed .row {
    border-left-color: var(--alert);
  }

  .meta {
    display: flex;
    gap: 10px;
    align-items: center;
    color: var(--ink-dim);
    font-size: 11px;
  }

  .code {
    color: var(--ink);
    font-variant-numeric: tabular-nums;
  }

  li.failed .code {
    color: var(--alert);
  }

  .cmd {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }

  .detail {
    padding: 0 12px 12px 14px;
  }

  .cwd {
    margin: 0 0 6px;
    color: var(--ink-dim);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  pre {
    margin: 0 0 8px;
    padding: 8px;
    background: var(--void);
    border: 1px solid var(--line);
    max-height: 240px;
    overflow: auto;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    font-family: var(--mono);
    font-size: 11.5px;
    color: var(--ink);
  }

  .muted {
    color: var(--ink-dim);
    margin: 0 0 6px;
  }

  .tag {
    border: 1px solid var(--line);
    color: var(--ink-dim);
    padding: 0 4px;
    font-size: 10px;
  }

  .fix {
    border-top: 1px solid var(--line);
    margin-top: 8px;
    padding-top: 8px;
  }

  .fix pre {
    margin-bottom: 6px;
    color: var(--signal);
  }

  .copy {
    padding: 4px 8px;
    background: none;
    border: 1px solid var(--line);
    color: var(--ink-dim);
    cursor: pointer;
    font: inherit;
    font-size: 11px;
  }

  .copy:hover {
    color: var(--ink);
    border-color: var(--signal);
  }

  .error {
    margin: 6px 12px;
    color: var(--alert);
  }

  .empty {
    padding: 16px 12px;
    color: var(--ink-dim);
    border: 0;
  }
</style>
