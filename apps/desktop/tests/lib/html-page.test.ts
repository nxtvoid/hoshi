import { describe, expect, test } from 'bun:test'
import {
  frameHeight,
  type HtmlPage,
  pageTheme,
  readHtmlPage,
  readPageMessage
} from '../../src/lib/html-page'

const output = (htmlRender: unknown) =>
  JSON.stringify({ htmlRender, message: 'Shown to the user.' })

describe('readHtmlPage', () => {
  test('reads the page and sorts its heights', () => {
    expect(
      readHtmlPage(
        output({
          id: 'abc_123',
          title: ' Sales ',
          height: 5000,
          heights: [
            [768, 300],
            [320, 40]
          ]
        })
      )
    ).toEqual({
      id: 'abc_123',
      title: 'Sales',
      height: 2000,
      heights: [
        [320, 80],
        [768, 300]
      ]
    })
  })

  test('refuses what no page could be', () => {
    expect(readHtmlPage(null)).toBeNull()
    expect(readHtmlPage('not json')).toBeNull()
    expect(readHtmlPage(output({ id: '../x', title: 'a', height: 1 }))).toBe(
      null
    )
    expect(readHtmlPage(output({ id: 'a_b', title: 'a' }))).toBeNull()
  })

  test('drops heights it cannot trust', () => {
    const page = readHtmlPage(
      output({ id: 'a_b', title: 'a', height: 300, heights: [[0, 1]] })
    )
    expect(page?.heights).toBeNull()
  })
})

describe('frameHeight', () => {
  const page: HtmlPage = {
    id: 'a_b',
    title: 'a',
    height: 400,
    heights: [
      [320, 600],
      [768, 300],
      [1000, 250]
    ]
  }

  test('follows the live page once it reported its height', () => {
    expect(frameHeight(page, 500, 1234.2)).toBe(1235)
    expect(frameHeight(page, 500, 10)).toBe(80)
  })

  test('takes the taller measurement around the width', () => {
    expect(frameHeight(page, 768, null)).toBe(300)
    expect(frameHeight(page, 900, null)).toBe(300)
    expect(frameHeight(page, 1000, null)).toBe(250)
    expect(frameHeight(page, 1200, null)).toBe(300)
  })

  test('keeps to the height the agent asked for when it asked for less', () => {
    const short = { ...page, height: 200 }
    expect(frameHeight(short, 320, null)).toBe(200)
    expect(frameHeight({ ...page, heights: null }, 320, null)).toBe(400)
  })
})

describe('readPageMessage', () => {
  test('reads heights and web links only', () => {
    const message = (method: string, params: unknown) => ({
      jsonrpc: '2.0',
      method,
      params
    })
    expect(
      readPageMessage(message('ui/notifications/size-changed', { height: 320 }))
    ).toEqual({ kind: 'height', height: 320 })
    expect(
      readPageMessage(message('ui/open-link', { url: 'https://a.dev' }))
    ).toEqual({ kind: 'link', url: 'https://a.dev' })
    expect(
      readPageMessage(message('ui/open-link', { url: 'file:///etc' }))
    ).toBeNull()
    expect(readPageMessage({ method: 'ui/open-link' })).toBeNull()
  })
})

describe('pageTheme', () => {
  test('maps app tokens and ends fonts in the system stack', () => {
    const theme = pageTheme(true, (name) =>
      name === '--font-sans'
        ? '"Geist Variable"'
        : name === '--send'
          ? 'blue'
          : ''
    )
    expect(theme.appearance).toBe('dark')
    expect(theme.variables['--accent']).toBe('blue')
    expect(theme.variables['--font-sans']).toStartWith('"Geist Variable", ')
    expect(theme.variables['--background']).toBeUndefined()
  })
})
