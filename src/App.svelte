<script lang="ts">
  import Workspace from './lib/Workspace.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { loadTheme } from './lib/theme.svelte'

  loadTheme()
</script>

<main class="layout">
  <Workspace />
  <Sidebar />
</main>

<style>
  .layout {
    height: 100%;
    display: flex;
    background: var(--void);
  }
</style>
