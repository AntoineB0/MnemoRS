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
      fontFamily: '"JetBrains Mono", "Fira Code", monospace',
      fontSize: 14,
      lineHeight: 1.2,
      cursorBlink: true,
      theme: {
        background: '#12131a',
        foreground: '#d6d6dd',
        cursor: '#c084fc',
        selectionBackground: '#3b3550',
      },
    })
    const fit = new FitAddon()
    term.loadAddon(fit)
    term.open(host)
    fit.fit()

    let sessionId = ''
    let ready = false

    const channel = new Channel<PtyEvent>()
    channel.onmessage = (event) => {
      if (event.type === 'data') term.write(event.chunk)
      else if (event.type === 'error')
        term.write(`\r\n\x1b[31m[pty] ${event.message}\x1b[0m\r\n`)
      else term.write(`\r\n\x1b[90m[processus terminé : ${event.code ?? '?'}]\x1b[0m\r\n`)
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
      .catch((error) => term.write(`\r\n\x1b[31m[spawn] ${error}\x1b[0m\r\n`))

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
      pendingResize = window.setTimeout(() => {
        fit.fit()
        if (ready) {
          invoke('pty_resize', { sessionId, cols: term.cols, rows: term.rows }).catch(() => {})
        }
      }, 50)
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
    min-height: 0;
    padding: 8px 4px 4px 10px;
    background: #12131a;
  }

  :global(.xterm) {
    height: 100%;
  }
</style>
