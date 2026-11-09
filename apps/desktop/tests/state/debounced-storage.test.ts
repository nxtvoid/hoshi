import { describe, expect, test } from 'bun:test'
import { debouncedStorage } from '../../src/state/debounced-storage'

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))

function memory() {
  const items = new Map<string, string>()
  let writes = 0
  return {
    items,
    writes: () => writes,
    backing: {
      getItem: (name: string) => items.get(name) ?? null,
      setItem: (name: string, value: string) => {
        writes += 1
        items.set(name, value)
      },
      removeItem: (name: string) => {
        items.delete(name)
      }
    }
  }
}

type Drafts = { drafts: Record<string, string> }

describe('debouncedStorage', () => {
  test('writes the latest value once after the delay', async () => {
    const store = memory()
    const storage = debouncedStorage<Drafts>(10, () => store.backing)
    storage.setItem('drafts', { state: { drafts: { a: 'h' } } })
    storage.setItem('drafts', { state: { drafts: { a: 'hi' } } })
    expect(store.writes()).toBe(0)
    await wait(30)
    expect(store.writes()).toBe(1)
    expect(JSON.parse(store.items.get('drafts') ?? '')).toEqual({
      state: { drafts: { a: 'hi' } }
    })
  })

  test('reads what waits to be written', () => {
    const store = memory()
    const storage = debouncedStorage<Drafts>(1000, () => store.backing)
    storage.setItem('drafts', { state: { drafts: { a: 'hi' } }, version: 0 })
    expect(storage.getItem('drafts')).toEqual({
      state: { drafts: { a: 'hi' } },
      version: 0
    })
    storage.flush()
  })

  test('flush writes at once', () => {
    const store = memory()
    const storage = debouncedStorage<Drafts>(1000, () => store.backing)
    storage.setItem('drafts', { state: { drafts: {} } })
    storage.flush()
    expect(store.writes()).toBe(1)
    storage.flush()
    expect(store.writes()).toBe(1)
  })

  test('reads what was written before', () => {
    const store = memory()
    store.items.set('drafts', JSON.stringify({ state: { drafts: { a: 'x' } } }))
    const storage = debouncedStorage<Drafts>(10, () => store.backing)
    expect(storage.getItem('drafts')).toEqual({ state: { drafts: { a: 'x' } } })
    expect(storage.getItem('missing')).toBeNull()
  })

  test('removing drops what waits', async () => {
    const store = memory()
    const storage = debouncedStorage<Drafts>(10, () => store.backing)
    storage.setItem('drafts', { state: { drafts: {} } })
    storage.removeItem('drafts')
    await wait(30)
    expect(store.items.has('drafts')).toBe(false)
  })
})
