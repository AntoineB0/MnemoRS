<script lang="ts">
  import Terminal from './lib/Terminal.svelte'
  import HistoryPanel from './lib/HistoryPanel.svelte'
</script>

<main class="layout">
  <Terminal />
  <HistoryPanel />
</main>

<style>
  .layout {
    height: 100%;
    display: flex;
    background: #12131a;
  }
</style>
