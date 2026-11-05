<script lang="ts">
  import HistoryPanel from './HistoryPanel.svelte'
  import ChatPanel from './ChatPanel.svelte'
  import SettingsPanel from './SettingsPanel.svelte'

  type Tab = 'history' | 'chat' | 'settings'

  let tab = $state<Tab>('history')

  const tabs: { id: Tab; label: string }[] = [
    { id: 'history', label: 'Historique' },
    { id: 'chat', label: 'Chat' },
    { id: 'settings', label: 'Réglages' },
  ]
</script>

<aside class="sidebar">
  <nav>
    {#each tabs as item (item.id)}
      <button class:active={tab === item.id} onclick={() => (tab = item.id)}>
        {item.label}
      </button>
    {/each}
  </nav>

  <div class="content">
    {#if tab === 'history'}
      <HistoryPanel />
    {:else if tab === 'chat'}
      <ChatPanel />
    {:else}
      <SettingsPanel />
    {/if}
  </div>
</aside>

<style>
  .sidebar {
    width: 360px;
    min-width: 300px;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-left: 1px solid var(--line);
  }

  nav {
    display: flex;
    border-bottom: 1px solid var(--line);
  }

  nav button {
    flex: 1;
    position: relative;
    padding: 11px 8px 10px;
    background: none;
    border: 0;
    color: var(--ink-dim);
    font-family: var(--mono);
    font-size: 12px;
    cursor: pointer;
  }

  nav button:hover {
    color: var(--ink);
  }

  nav button.active {
    color: var(--ink);
  }

  nav button.active::after {
    content: '';
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: -1px;
    height: 2px;
    background: var(--signal);
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
