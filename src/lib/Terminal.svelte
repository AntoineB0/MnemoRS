<script lang="ts">
  import { onMount } from 'svelte'
  import { Channel, invoke } from '@tauri-apps/api/core'
  import { Terminal as Xterm } from '@xterm/xterm'
  import { FitAddon } from '@xterm/addon-fit'
  import '@xterm/xterm/css/xterm.css'

  type PtyEvent =
    | { type: 'data'; chunk: string }
    | { type: 'exit'; code: number | null }
    | { type: 'error'; message: string }

  let host: HTMLDivElement

  onMount(() => {
    const term = new Xterm({
      fontFamily: "'JetBrainsMono Nerd Font Mono', 'JetBrains Mono', monospace",
      fontSize: 14,
      lineHeight: 1.25,
      cursorBlink: true,
      cursorStyle: 'block',
      theme: {
        background: '#252926',
        foreground: '#f2ede0',
        cursor: '#00ff88',
        cursorAccent: '#252926',
        selectionBackground: '#3a403b',
        black: '#252926',
        red: '#ff8a73',
        green: '#00ff88',
        yellow: '#e8d48b',
        blue: '#7fd1c1',
        magenta: '#e0a8c8',
        cyan: '#8fe3d8',
        white: '#f2ede0',
        brightBlack: '#6b736c',
        brightRed: '#ffab99',
        brightGreen: '#5cffb0',
        brightYellow: '#f2e3a8',
        brightBlue: '#a3e3d6',
        brightMagenta: '#f0c6dc',
        brightCyan: '#b8f0e8',
        brightWhite: '#ffffff',
      },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host)

    let sessionId = ''
    let ready = false

    const refit = () => {
      fit.fit()
      if (ready && term.cols > 2) {
        invoke('pty_resize', { sessionId, cols: term.cols, rows: term.rows }).catch(() => {})
      }
    }

    refit()
    requestAnimationFrame(refit)
    document.fonts?.ready.then(refit).catch(() => {})

    const channel = new Channel<PtyEvent>()
    channel.onmessage = (event) => {
      if (event.type === 'data') term.write(event.chunk)
      else if (event.type === 'error')
        term.write(`\r\n\x1b[38;2;255;138;115m[pty] ${event.message}\x1b[0m\r\n`)
      else
        term.write(
          `\r\n\x1b[38;2;169;177;170m[processus terminé : ${event.code ?? '?'}]\x1b[0m\r\n`,
        )
    }

    invoke<string>('pty_spawn', {
      cols: term.cols,
      rows: term.rows,
      onEvent: channel,
    })
      .then((id) => {
        sessionId = id
        ready = true
        term.focus()
      })
      .catch((error) =>
        term.write(`\r\n\x1b[38;2;255;138;115m[spawn] ${error}\x1b[0m\r\n`),
      )

    const dataListener = term.onData((data) => {
      if (!ready) return
      invoke('pty_write', { sessionId, data }).catch(() => {})
    })

    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown' || !event.ctrlKey || !event.shiftKey) return true
      const key = event.key.toLowerCase()
      if (key === 'c' && term.hasSelection()) {
        navigator.clipboard.writeText(term.getSelection()).catch(() => {})
        return false
      }
      if (key === 'v') {
        navigator.clipboard.readText().then((text) => term.paste(text)).catch(() => {})
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

    const focus = () => term.focus()
    host.addEventListener('click', focus)
    term.focus()

    return () => {
      observer.disconnect()
      host.removeEventListener('click', focus)
      dataListener.dispose()
      if (sessionId) invoke('pty_kill', { sessionId }).catch(() => {})
      term.dispose()
    }
  })
</script>

<div class="terminal" bind:this={host}></div>

<style>
  .terminal {
    flex: 1;
    min-width: 0;
    min-height: 0;
    padding: 10px 6px 6px 12px;
    background: var(--void);
  }

  :global(.xterm) {
    height: 100%;
  }
</style>
