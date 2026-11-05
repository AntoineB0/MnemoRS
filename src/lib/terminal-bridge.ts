type InsertHandler = (text: string) => void

let insert: InsertHandler | null = null

export function registerTerminalInsert(handler: InsertHandler) {
  insert = handler
  return () => {
    if (insert === handler) insert = null
  }
}

export function insertIntoTerminal(text: string) {
  insert?.(text)
}
