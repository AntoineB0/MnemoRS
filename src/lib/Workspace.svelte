<script lang="ts">
  import Terminal from './Terminal.svelte'

  type Tab = { key: number; title: string }
  type Pane = { key: number; tabs: Tab[]; active: number }

  let nextTab = 1
  let nextPane = 1

  function makeTab(): Tab {
    const key = nextTab++
    return { key, title: `Terminal ${key}` }
  }

  function makePane(): Pane {
    const tab = makeTab()
    return { key: nextPane++, tabs: [tab], active: tab.key }
  }

  let panes = $state<Pane[]>([makePane()])
  let activePane = $state(panes[0].key)

  function findPane(key: number) {
    return panes.find((pane) => pane.key === key)
  }

  function selectTab(pane: Pane, key: number) {
    pane.active = key
    activePane = pane.key
  }

  function addTab(paneKey: number) {
    const pane = findPane(paneKey)
    if (!pane) return
    const tab = makeTab()
    pane.tabs.push(tab)
    selectTab(pane, tab.key)
  }

  function closeTab(paneKey: number, tabKey: number) {
    const pane = findPane(paneKey)
    if (!pane) return
    const index = pane.tabs.findIndex((tab) => tab.key === tabKey)
    if (index === -1) return

    pane.tabs.splice(index, 1)
    if (pane.tabs.length === 0) {
      closePane(paneKey)
      return
    }
    if (pane.active === tabKey) {
      pane.active = pane.tabs[Math.max(0, index - 1)].key
    }
  }

  function splitPane() {
    if (panes.length >= 2) return
    const pane = makePane()
    panes.push(pane)
    activePane = pane.key
  }

  function closePane(paneKey: number) {
    if (panes.length === 1) return
    const index = panes.findIndex((pane) => pane.key === paneKey)
    if (index === -1) return
    panes.splice(index, 1)
    if (activePane === paneKey) activePane = panes[0].key
  }
</script>

<div class="workspace" class:split={panes.length > 1}>
  {#each panes as pane (pane.key)}
    <section class="pane">
      <header class="tabs">
        {#each pane.tabs as tab (tab.key)}
          <div class="tab" class:active={tab.key === pane.active}>
            <button class="label" onclick={() => selectTab(pane, tab.key)}>
              {tab.title}
            </button>
            <button
              class="close"
              title="fermer l'onglet"
              disabled={panes.length === 1 && pane.tabs.length === 1}
              onclick={() => closeTab(pane.key, tab.key)}
            >
              ×
            </button>
          </div>
        {/each}
        <button class="icon" title="nouvel onglet" onclick={() => addTab(pane.key)}>+</button>
        <span class="spacer"></span>
        {#if panes.length < 2}
          <button class="icon" title="découper l'affichage" onclick={splitPane}>
            ◫
          </button>
        {:else}
          <button class="icon" title="fermer le panneau" onclick={() => closePane(pane.key)}>
            ×
          </button>
        {/if}
      </header>

      <div class="body">
        {#each pane.tabs as tab (tab.key)}
          <div class="slot" class:hidden={tab.key !== pane.active}>
            <Terminal
              active={tab.key === pane.active && pane.key === activePane}
              onactivate={() => (activePane = pane.key)}
            />
          </div>
        {/each}
      </div>
    </section>
  {/each}
</div>

<style>
  .workspace {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    background: var(--void);
  }

  /* Le séparateur passe dans l'espace entre les panneaux : aucune bordure
     n'ajoute de décalage vertical à la barre d'onglets. */
  .workspace.split {
    gap: 2px;
    background: var(--line);
  }

  .pane {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--void);
  }

  .tabs {
    display: flex;
    align-items: stretch;
    height: var(--topbar);
    background: var(--surface);
    border-bottom: 1px solid var(--line);
    font-family: var(--mono);
    font-size: 11.5px;
  }

  .tab {
    display: flex;
    align-items: stretch;
    border-right: 1px solid var(--line);
    color: var(--ink-dim);
  }

  .tab.active {
    background: var(--void);
    color: var(--signal);
  }

  .tab .label {
    display: flex;
    align-items: center;
    padding: 0 8px;
    background: none;
    border: 0;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .tab .close {
    display: flex;
    align-items: center;
    padding: 0 6px;
    background: none;
    border: 0;
    color: var(--ink-faint);
    font: inherit;
    cursor: pointer;
  }

  .tab .close:hover:not(:disabled) {
    color: var(--alert);
  }

  .tab .close:disabled {
    opacity: 0.3;
    cursor: default;
  }

  .icon {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    background: none;
    border: 0;
    color: var(--ink-dim);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
  }

  .icon:hover {
    color: var(--ink);
  }

  .spacer {
    flex: 1;
  }

  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }

  .slot {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }

  .slot.hidden {
    display: none;
  }
</style>
