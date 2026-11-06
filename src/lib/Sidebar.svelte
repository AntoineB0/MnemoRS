<script lang="ts">
  import HistoryPanel from './HistoryPanel.svelte'
  import ChatPanel from './ChatPanel.svelte'
  import SettingsPanel from './SettingsPanel.svelte'

  type Tab = 'history' | 'chat' | 'settings'

  let { width = 360 }: { width?: number } = $props()

  let tab = $state<Tab>('history')

  const tabs: { id: Tab; label: string }[] = [
    { id: 'history', label: 'Historique' },
    { id: 'chat', label: 'Chat' },
    { id: 'settings', label: 'Réglages' },
  ]
</script>

<aside class="sidebar" style:width={`${width}px`}>
  <nav>
    {#each tabs as item (item.id)}
      <button class:active={tab === item.id} onclick={() => (tab = item.id)}>
        {item.label}
      </button>
    {/each}
  </nav>

  <div class="content">
    <!-- Les panneaux restent montés : changer d'onglet ne perd ni la
         conversation en cours ni la position de lecture. -->
    <div class="panel" class:hidden={tab !== 'history'}>
      <HistoryPanel />
    </div>
    <div class="panel" class:hidden={tab !== 'chat'}>
      <ChatPanel visible={tab === 'chat'} />
    </div>
    <div class="panel" class:hidden={tab !== 'settings'}>
      <SettingsPanel />
    </div>
  </div>
</aside>

<style>
  .sidebar {
    flex: none;
    max-width: calc(100vw - 320px);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    border-left: 1px solid var(--line);
  }

  nav {
    display: flex;
    height: var(--topbar);
    border-bottom: 1px solid var(--line);
  }

  nav button {
    flex: 1;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 8px;
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

  .panel {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }

  .panel.hidden {
    display: none;
  }
</style>
