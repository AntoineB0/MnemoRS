<script lang="ts">
  import { onMount } from 'svelte'

  let { visible = true }: { visible?: boolean } = $props()
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { openUrl } from '@tauri-apps/plugin-opener'
  import MarkdownIt from 'markdown-it'
  import { insertIntoTerminal } from './terminal-bridge'

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
  let hint = $state('')
  let hintTimer: number | undefined
  let requestId = ''
  let list: HTMLDivElement

  const current = $derived(providers.find((p) => p.id === providerId))

  // html désactivé : le modèle ne peut pas injecter de balises dans la WebView.
  const md = new MarkdownIt({ html: false, linkify: true, breaks: true })

  function renderMarkdown(text: string) {
    return md.render(text)
  }

  function showHint(text: string) {
    hint = text
    clearTimeout(hintTimer)
    hintTimer = window.setTimeout(() => (hint = ''), 2000)
  }

  function handleClick(event: MouseEvent) {
    const target = event.target as HTMLElement

    const anchor = target.closest('.messages a')
    if (anchor) {
      event.preventDefault()
      const href = anchor.getAttribute('href')
      if (href && /^https?:\/\//i.test(href)) openUrl(href).catch(() => {})
      return
    }

    const code = target.closest('.messages pre code')
    if (!code) return
    if (window.getSelection()?.toString().trim()) return

    const text = (code.textContent ?? '').trim().replace(/^\$\s+/, '')
    if (!text) return

    if (event.ctrlKey || event.metaKey) {
      if (text.includes('\n')) {
        navigator.clipboard.writeText(text).catch(() => {})
        showHint('Commande multi-ligne : copiée, pas insérée.')
        return
      }
      insertIntoTerminal(text)
      showHint('Commande insérée dans le terminal.')
      return
    }

    navigator.clipboard
      .writeText(text)
      .then(() => showHint('Commande copiée.'))
      .catch(() => showHint('Presse-papiers indisponible.'))
  }

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

  onMount(() => {
    void loadProviders()
    document.addEventListener('click', handleClick)
    return () => document.removeEventListener('click', handleClick)
  })

  $effect(() => {
    const count = messages.length
    const lastLength = messages[count - 1]?.content.length ?? 0
    if (!visible || !list) return
    list.scrollTop = list.scrollHeight
    void lastLength
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
    } finally {
      // Garde-fou : même si l'événement final se perd, l'envoi redevient possible.
      streaming = false
      requestId = ''
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
    <div class="select">
      <select bind:value={providerId} disabled={streaming}>
        {#each providers as provider (provider.id)}
          <option value={provider.id}>{provider.name} · {provider.model}</option>
        {:else}
          <option value="">aucun provider</option>
        {/each}
      </select>
      <span class="chevron"></span>
    </div>
    <button onclick={clear} disabled={streaming || messages.length === 0}>effacer</button>
  </div>

  <div class="messages" role="log" aria-live="polite" bind:this={list}>
    {#each messages as message, index (index)}
      <article class={message.role}>
        <span class="role">{message.role === 'user' ? 'vous' : 'ia'}</span>
        <div class="text">
          {#if message.content}
            {@html renderMarkdown(message.content)}
          {:else if streaming}
            <p>…</p>
          {/if}
        </div>
      </article>
    {:else}
      <p class="muted">Pose une question sur tes commandes.</p>
    {/each}
  </div>

  {#if hint}
    <p class="hint">{hint}</p>
  {/if}

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

  .select {
    position: relative;
    flex: 1;
    min-width: 0;
    display: flex;
  }

  .select select {
    flex: 1;
    min-width: 0;
    padding: 5px 26px 5px 6px;
    font-size: 11.5px;
    appearance: none;
    -webkit-appearance: none;
  }

  .chevron {
    position: absolute;
    right: 9px;
    top: 50%;
    width: 6px;
    height: 6px;
    border-right: 1.5px solid var(--ink-dim);
    border-bottom: 1.5px solid var(--ink-dim);
    transform: translateY(-70%) rotate(45deg);
    pointer-events: none;
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
    color: var(--ink-faint);
    border-color: var(--line);
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
    font-family: var(--mono);
    font-size: 13.5px;
    line-height: 1.55;
    overflow-wrap: anywhere;
    color: var(--ink);
  }

  .text :global(p) {
    margin: 0 0 8px;
  }

  .text :global(p:last-child) {
    margin-bottom: 0;
  }

  .text :global(ul),
  .text :global(ol) {
    margin: 0 0 8px;
    padding-left: 20px;
  }

  .text :global(li) {
    margin: 2px 0;
  }

  .text :global(code) {
    font-family: var(--mono);
    font-size: 12px;
    background: var(--void);
    border: 1px solid var(--line);
    padding: 0 3px;
  }

  .text :global(pre) {
    position: relative;
    margin: 0 0 8px;
    padding: 8px;
    background: var(--void);
    border: 1px solid var(--line);
    overflow-x: auto;
    cursor: copy;
  }

  .text :global(pre:hover)::after {
    content: 'clic pour copier, ctrl+clic vers le terminal';
    position: absolute;
    top: 3px;
    right: 6px;
    padding: 0 4px;
    background: var(--void);
    color: var(--ink-faint);
    font-family: var(--mono);
    font-size: 10px;
    pointer-events: none;
  }

  .text :global(pre code) {
    border: 0;
    padding: 0;
    background: none;
  }

  .text :global(h1),
  .text :global(h2),
  .text :global(h3),
  .text :global(h4) {
    font-family: var(--mono);
    font-size: 13px;
    font-weight: 600;
    margin: 12px 0 6px;
  }

  .text :global(a) {
    color: var(--signal);
    text-decoration: underline;
  }

  .text :global(blockquote) {
    margin: 0 0 8px;
    padding-left: 10px;
    border-left: 2px solid var(--line);
    color: var(--ink-dim);
  }

  .text :global(hr) {
    border: 0;
    border-top: 1px solid var(--line);
    margin: 12px 0;
  }

  .text :global(table) {
    border-collapse: collapse;
    margin: 0 0 8px;
    font-size: 12.5px;
  }

  .text :global(th),
  .text :global(td) {
    border: 1px solid var(--line);
    padding: 4px 6px;
    text-align: left;
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
    color: var(--ink-faint);
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

  .hint {
    margin: 0 12px 8px;
    color: var(--ink-dim);
    font-size: 11px;
  }

  .error {
    margin: 0 12px 8px;
    color: var(--alert);
    font-size: 12px;
  }
</style>
