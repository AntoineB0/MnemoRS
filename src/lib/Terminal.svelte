<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { Terminal as Xterm } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import '@xterm/xterm/css/xterm.css'
  import { theme, terminalTheme } from './theme.svelte'
  import { registerTerminalInsert } from './terminal-bridge'

  type PtyEvent =
    | { type: 'data'; chunk: string }
    | { type: 'exit'; code: number | null }
    | { type: 'error'; message: string }

  let host: HTMLDivElement
  let term: Xterm | undefined
  let exited = $state(false)
  let restart = $state<() => void>(() => {})

  $effect(() => {
    if (term) term.options.theme = terminalTheme(theme.current)
  })

  onMount(() => {
    const terminal = new Xterm({
      fontFamily: "'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', monospace",
      fontSize: 14,
      lineHeight: 1.25,
      cursorBlink: true,
      cursorStyle: 'block',
      theme: terminalTheme(theme.current),
    })
    term = terminal
    const fit = new FitAddon()
    terminal.loadAddon(fit)
    terminal.open(host)

    let sessionId = ''
    let ready = false

    const unregister = registerTerminalInsert((text) => {
      if (!ready) return
      invoke('pty_write', { sessionId, data: text }).catch(() => {})
      terminal.focus()
    })

    const refit = () => {
      fit.fit()
      if (ready && terminal.cols > 2) {
        invoke('pty_resize', { sessionId, cols: terminal.cols, rows: terminal.rows }).catch(() => {})
      }
    }

    refit()
    requestAnimationFrame(refit)
    document.fonts?.ready.then(refit).catch(() => {})

    const channel = new Channel<PtyEvent>()
    channel.onmessage = (event) => {
      if (event.type === 'data') terminal.write(event.chunk)
      else if (event.type === 'error')
        terminal.write(`\r\n\x1b[31m[pty] ${event.message}\x1b[0m\r\n`)
      else {
        ready = false
        exited = true
        terminal.write(
          `\r\n\x1b[90m[processus terminé : ${event.code ?? '?'}]\x1b[0m\r\n`,
        )
      }
    }

    const start = () => {
      exited = false
      invoke<string>('pty_spawn', {
        cols: terminal.cols,
        rows: terminal.rows,
        onEvent: channel,
      })
        .then((id) => {
          sessionId = id
          ready = true
          terminal.focus()
        })
        .catch((error) =>
          terminal.write(`\r\n\x1b[31m[spawn] ${error}\x1b[0m\r\n`),
        )
    }

    restart = () => {
      if (sessionId) invoke('pty_kill', { sessionId }).catch(() => {})
      sessionId = ''
      start()
    }

    start()

    const dataListener = terminal.onData((data) => {
      if (!ready) return
      invoke('pty_write', { sessionId, data }).catch(() => {})
    })

    terminal.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown' || !event.ctrlKey || !event.shiftKey) return true
      const key = event.key.toLowerCase()
      if (key === 'c' && terminal.hasSelection()) {
        navigator.clipboard.writeText(terminal.getSelection()).catch(() => {})
        return false
      }
      if (key === 'v') {
        navigator.clipboard.readText().then((text) => terminal.paste(text)).catch(() => {})
        return false
      }
      return true
    })

    let pendingResize: number | undefined
    const observer = new ResizeObserver(() => {
      window.clearTimeout(pendingResize)
      pendingResize = window.setTimeout(refit, 50)
    })
    observer.observe(host)

    const focus = () => terminal.focus()
    host.addEventListener('click', focus)
    terminal.focus()

    return () => {
      unregister()
      observer.disconnect()
      host.removeEventListener('click', focus)
      dataListener.dispose()
      if (sessionId) invoke('pty_kill', { sessionId }).catch(() => {})
      terminal.dispose()
      term = undefined
    }
  })
</script>

<div class="terminal-wrap">
  <div class="terminal" bind:this={host}></div>
  {#if exited}
    <button class="restart" onclick={restart}>relancer le terminal</button>
  {/if}
</div>

<style>
  .terminal-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }

  .terminal {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 10px 6px 6px 12px;
    background: var(--void);
  }

  .restart {
    position: absolute;
    right: 16px;
    bottom: 16px;
    padding: 6px 12px;
    background: var(--surface-2);
    border: 1px solid var(--line);
    color: var(--ink);
    font-family: var(--mono);
    font-size: 12px;
    cursor: pointer;
  }

  .restart:hover {
    border-color: var(--signal);
    color: var(--signal);
  }

  :global(.xterm) {
    height: 100%;
  }
</style>
