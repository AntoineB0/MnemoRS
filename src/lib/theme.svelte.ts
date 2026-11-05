export type ThemeMode = 'dark' | 'light'

export type Theme = {
  id: string
  name: string
  mode: ThemeMode
  void: string
  surface: string
  surface2: string
  line: string
  ink: string
  inkDim: string
  inkFaint: string
  signal: string
  alert: string
}

const ACTIVE_KEY = 'mnemors.theme.active'
const CUSTOM_KEY = 'mnemors.theme.custom'

export const presets: Theme[] = [
  {
    id: 'mnemo',
    name: 'Mnemo',
    mode: 'dark',
    void: '#252926',
    surface: '#1f2320',
    surface2: '#2b302c',
    line: '#3a403b',
    ink: '#f2ede0',
    inkDim: '#a9b1aa',
    inkFaint: '#6b736c',
    signal: '#00ff88',
    alert: '#ff8a73',
  },
  {
    id: 'mnemo-vert',
    name: 'Mnemo vert',
    mode: 'dark',
    void: '#00241e',
    surface: '#003a30',
    surface2: '#00483c',
    line: '#0c5b4b',
    ink: '#f2ede0',
    inkDim: '#9fb8ae',
    inkFaint: '#5f7a71',
    signal: '#00ff88',
    alert: '#ff8a73',
  },
  {
    id: 'papier',
    name: 'Papier',
    mode: 'light',
    void: '#f2ede0',
    surface: '#e7e1d3',
    surface2: '#dcd5c5',
    line: '#c4bda9',
    ink: '#1f2320',
    inkDim: '#5c635c',
    inkFaint: '#8b928b',
    signal: '#007a4d',
    alert: '#b23a2a',
  },
]

export const theme = $state<{ current: Theme; custom: Theme[] }>({
  current: presets[0],
  custom: [],
})

const ANSI_DARK = {
  yellow: '#e8d48b',
  blue: '#7fd1c1',
  magenta: '#e0a8c8',
  cyan: '#8fe3d8',
  brightYellow: '#f2e3a8',
  brightBlue: '#a3e3d6',
  brightMagenta: '#f0c6dc',
  brightCyan: '#b8f0e8',
}

const ANSI_LIGHT = {
  yellow: '#7a5c12',
  blue: '#1f6f63',
  magenta: '#8a3f6b',
  cyan: '#1f6f7a',
  brightYellow: '#9a7718',
  brightBlue: '#2a8a7c',
  brightMagenta: '#a85586',
  brightCyan: '#2a8a96',
}

export function terminalTheme(t: Theme) {
  const ansi = t.mode === 'dark' ? ANSI_DARK : ANSI_LIGHT
  const dark = t.mode === 'dark'
  return {
    background: t.void,
    foreground: t.ink,
    cursor: t.signal,
    cursorAccent: t.void,
    selectionBackground: t.line,
    black: dark ? t.void : t.ink,
    red: t.alert,
    green: t.signal,
    yellow: ansi.yellow,
    blue: ansi.blue,
    magenta: ansi.magenta,
    cyan: ansi.cyan,
    white: dark ? t.ink : t.surface,
    brightBlack: t.inkDim,
    brightRed: t.alert,
    brightGreen: t.signal,
    brightYellow: ansi.brightYellow,
    brightBlue: ansi.brightBlue,
    brightMagenta: ansi.brightMagenta,
    brightCyan: ansi.brightCyan,
    brightWhite: dark ? '#ffffff' : t.ink,
  }
}

export function applyTheme(t: Theme, persist = true) {
  const root = document.documentElement
  const vars: Record<string, string> = {
    '--void': t.void,
    '--surface': t.surface,
    '--surface-2': t.surface2,
    '--line': t.line,
    '--ink': t.ink,
    '--ink-dim': t.inkDim,
    '--ink-faint': t.inkFaint,
    '--signal': t.signal,
    '--alert': t.alert,
  }
  for (const [key, value] of Object.entries(vars)) {
    root.style.setProperty(key, value)
  }
  root.style.colorScheme = t.mode
  theme.current = t
  if (persist) {
    localStorage.setItem(ACTIVE_KEY, JSON.stringify(t))
  }
}

export function loadTheme() {
  try {
    const custom = JSON.parse(localStorage.getItem(CUSTOM_KEY) ?? '[]')
    if (Array.isArray(custom)) theme.custom = custom
  } catch {
    theme.custom = []
  }

  try {
    const saved = JSON.parse(localStorage.getItem(ACTIVE_KEY) ?? 'null')
    applyTheme(isTheme(saved) ? saved : presets[0], false)
  } catch {
    applyTheme(presets[0], false)
  }
}

export function saveCustom(t: Theme) {
  const name = t.name.trim() || 'Sans nom'
  const custom: Theme = { ...t, id: `custom-${Date.now()}`, name }
  theme.custom = [...theme.custom.filter((item) => item.name !== name), custom]
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(theme.custom))
  applyTheme(custom)
}

export function deleteCustom(id: string) {
  theme.custom = theme.custom.filter((item) => item.id !== id)
  localStorage.setItem(CUSTOM_KEY, JSON.stringify(theme.custom))
}

function isTheme(value: unknown): value is Theme {
  if (!value || typeof value !== 'object') return false
  const keys = ['void', 'surface', 'surface2', 'line', 'ink', 'inkDim', 'inkFaint', 'signal', 'alert']
  return keys.every((key) => typeof (value as Record<string, unknown>)[key] === 'string')
}
