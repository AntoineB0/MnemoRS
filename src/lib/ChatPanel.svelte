<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'

  type ProviderView = {
    id: string
    name: string
    baseUrl: string
    model: string
    hasKey: boolean
  }

  type ChatEvent =
    | { type: 'delta'; text: string }
    | { type: 'done' }
    | { type: 'cancelled' }
    | { type: 'error'; message: string }

  type Message = { role: 'user' | 'assistant'; content: string }

  let providers = $state<ProviderView[]>([])
  let providerId = $state('')
  let messages = $state<Message[]>([])
  let input = $state('')
  let streaming = $state(false)
  let error = $state('')
  let requestId = ''
  let list: HTMLDivElement

  const current = $derived(providers.find((p) => p.id === providerId))

  async function loadProviders() {
    try {
      providers = await invoke<ProviderView[]>('providers_list')
      if (!providers.some((p) => p.id === providerId)) {
        providerId = providers[0]?.id ?? ''
      }
    } catch (e) {
      error = String(e)
    }
  }

  onMount(loadProviders)

  $effect(() => {
    messages.length
    if (list) list.scrollTop = list.scrollHeight
  })

  async function send() {
    const text = input.trim()
    if (!text || streaming) return
    if (!current) {
      error = 'Aucun provider configuré.'
      return
    }

    error = ''
    input = ''
    messages = [...messages, { role: 'user', content: text }, { role: 'assistant', content: '' }]
    streaming = true
    requestId = crypto.randomUUID()

    const channel = new Channel<ChatEvent>()
    channel.onmessage = (event) => {
      if (event.type === 'delta') {
        messages[messages.length - 1].content += event.text
        return
      }
      streaming = false
      if (event.type === 'error') error = event.message
    }

    try {
      await invoke('chat_send', {
        requestId,
        providerId,
        messages: messages.slice(0, -1),
        onEvent: channel,
      })
    } catch (e) {
      error = String(e)
      streaming = false
    }
  }

  function stop() {
    if (requestId) invoke('chat_cancel', { requestId }).catch(() => {})
  }

  function keydown(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      void send()
    }
  }

  function clear() {
    messages = []
    error = ''
  }
</script>

<div class="chat">
  <div class="bar">
    <select bind:value={providerId} disabled={streaming}>
      {#each providers as provider (provider.id)}
        <option value={provider.id}>{provider.name} · {provider.model}</option>
      {:else}
        <option value="">aucun provider</option>
      {/each}
    </select>
    <button onclick={clear} disabled={streaming || messages.length === 0}>effacer</button>
  </div>

  <div class="messages" bind:this={list}>
    {#each messages as message, index (index)}
      <article class={message.role}>
        <span class="role">{message.role === 'user' ? 'vous' : 'ia'}</span>
        <span class="text">{message.content || (streaming ? '…' : '')}</span>
      </article>
    {:else}
      <p class="muted">Pose une question sur tes commandes.</p>
    {/each}
  </div>

  {#if error}
    <p class="error">{error}</p>
  {/if}

  <div class="compose">
    <textarea
      bind:value={input}
      onkeydown={keydown}
      placeholder="Entrée pour envoyer, Maj+Entrée pour un saut de ligne"
      rows="2"
    ></textarea>
    {#if streaming}
      <button class="stop" onclick={stop}>stop</button>
    {:else}
      <button class="send" onclick={send} disabled={!current || !input.trim()}>envoyer</button>
    {/if}
  </div>
</div>

<style>
  .chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    font-family: var(--mono);
  }

  .bar {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 10px 12px;
    border-bottom: 1px solid var(--line);
  }

  select,
  button,
  textarea {
    font-family: var(--mono);
    color: var(--ink);
    background: transparent;
    border: 1px solid var(--line);
  }

  select {
    flex: 1;
    min-width: 0;
    padding: 5px 6px;
    font-size: 11.5px;
  }

  button {
    padding: 5px 10px;
    font-size: 11px;
    cursor: pointer;
  }

  button:hover:not(:disabled) {
    border-color: var(--signal);
    color: var(--signal);
  }

  button:disabled {
    color: #5f7a71;
    border-color: #0a4a3d;
    cursor: default;
  }

  .messages {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  article {
    display: flex;
    flex-direction: column;
    gap: 5px;
    padding-left: 10px;
    border-left: 2px solid var(--line);
  }

  article.user {
    border-left-color: var(--signal);
  }

  .role {
    font-size: 10px;
    color: var(--ink-dim);
  }

  .text {
    font-family: var(--sans);
    font-size: 13.5px;
    line-height: 1.55;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--ink);
  }

  .compose {
    display: flex;
    gap: 8px;
    padding: 10px 12px 12px;
    border-top: 1px solid var(--line);
  }

  textarea {
    flex: 1;
    resize: none;
    padding: 7px 8px;
    font-size: 12px;
    line-height: 1.45;
    background: var(--void);
  }

  textarea::placeholder {
    color: #5f7a71;
  }

  textarea:focus,
  select:focus {
    outline: 1px solid var(--signal);
    outline-offset: 0;
  }

  .send {
    border-color: var(--signal);
    color: var(--signal);
  }

  .stop {
    border-color: var(--alert);
    color: var(--alert);
  }

  .muted {
    color: var(--ink-dim);
    font-size: 12px;
  }

  .error {
    margin: 0 12px 8px;
    color: var(--alert);
    font-size: 12px;
  }
</style>
