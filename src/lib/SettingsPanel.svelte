<script lang="ts">
  import { onMount } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'

  type ProviderView = {
    id: string
    name: string
    baseUrl: string
    model: string
    hasKey: boolean
  }

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

  onMount(load)

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

  .hint {
    color: #6b6b7f;
    font-size: 11px;
    line-height: 1.4;
  }
</style>
