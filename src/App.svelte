<script lang="ts">
  import Terminal from './lib/Terminal.svelte'
  import Sidebar from './lib/Sidebar.svelte'
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
