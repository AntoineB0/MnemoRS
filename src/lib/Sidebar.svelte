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
    width: 380px;
    min-width: 300px;
    display: flex;
    flex-direction: column;
    background: #171821;
    border-left: 1px solid #2a2b38;
    color: #d6d6dd;
  }

  nav {
    display: flex;
    border-bottom: 1px solid #2a2b38;
  }

  nav button {
    flex: 1;
    padding: 9px 4px;
    background: none;
    border: 0;
    border-bottom: 2px solid transparent;
    color: #9b9bab;
    font: 600 11px/1 system-ui, sans-serif;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    cursor: pointer;
  }

  nav button.active {
    color: #e9d5ff;
    border-bottom-color: #c084fc;
  }

  .content {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
