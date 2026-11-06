<script lang="ts">
  import Workspace from './lib/Workspace.svelte'
  import Sidebar from './lib/Sidebar.svelte'
  import { loadTheme } from './lib/theme.svelte'

  loadTheme()

  const MIN_WIDTH = 300
  const MAX_WIDTH = 720
  const WIDTH_KEY = 'mnemors.sidebar.width'

  let width = $state(loadWidth())
  let dragging = $state(false)

  function loadWidth() {
    try {
      const saved = Number(localStorage.getItem(WIDTH_KEY))
      if (Number.isFinite(saved) && saved >= MIN_WIDTH) return clamp(saved)
    } catch {
      // localStorage indisponible : on garde la largeur par défaut.
    }
    return 360
  }

  function clamp(value: number) {
    // Le terminal garde toujours au moins 320 px, même sur une fenêtre étroite.
    const max = Math.max(MIN_WIDTH, Math.min(MAX_WIDTH, window.innerWidth - 320))
    return Math.min(max, Math.max(MIN_WIDTH, value))
  }

  function save(value: number) {
    width = value
    try {
      localStorage.setItem(WIDTH_KEY, String(value))
    } catch {
      // Sans stockage, le redimensionnement reste valable pour la session.
    }
  }

  function startDrag(event: PointerEvent) {
    dragging = true
    ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  }

  function drag(event: PointerEvent) {
    if (!dragging) return
    width = clamp(window.innerWidth - event.clientX)
  }

  function stopDrag() {
    if (!dragging) return
    dragging = false
    // Écriture unique en fin de geste : localStorage est synchrone.
    save(width)
  }

  function keydown(event: KeyboardEvent) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    event.preventDefault()
    save(clamp(width + (event.key === 'ArrowLeft' ? 20 : -20)))
  }
</script>

<main class="layout" class:dragging>
  <Workspace />
  <!-- role="separator" est le motif ARIA d'une poignée de redimensionnement ;
       le vérificateur la classe à tort comme non interactive. -->
  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="resizer"
    class:active={dragging}
    role="separator"
    aria-orientation="vertical"
    aria-label="redimensionner le panneau latéral"
    aria-valuemin={MIN_WIDTH}
    aria-valuemax={MAX_WIDTH}
    aria-valuenow={width}
    tabindex="0"
    title="glisser pour redimensionner"
    onpointerdown={startDrag}
    onpointermove={drag}
    onpointerup={stopDrag}
    onpointercancel={stopDrag}
    onkeydown={keydown}
  ></div>
  <Sidebar {width} />
</main>

<style>
  .layout {
    height: 100%;
    display: flex;
    background: var(--void);
  }

  .layout.dragging {
    user-select: none;
    cursor: col-resize;
  }

  .resizer {
    flex: none;
    width: 4px;
    background: var(--line);
    cursor: col-resize;
    touch-action: none;
  }

  .resizer:hover,
  .resizer.active {
    background: var(--signal);
  }

  .resizer:focus-visible {
    outline: 1px solid var(--signal);
    outline-offset: 0;
  }
</style>
