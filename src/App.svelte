<script lang="ts">
  import Terminal from './lib/Terminal.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { loadTheme } from './lib/theme.svelte'

  loadTheme()
</script>

<main class="layout">
  <Terminal />
  <Sidebar />
</main>

<style>
  .layout {
    height: 100%;
    display: flex;
    background: var(--void);
  }
</style>
