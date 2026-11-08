import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { Markdown } from '../../src/components/markdown/Markdown'

const render = (text: string, source?: 'agent' | 'forge') =>
  renderToStaticMarkup(<Markdown text={text} source={source} />)

describe('Markdown', () => {
  test('renders the HTML a bot writes in a comment, as GitHub does', () => {
    const html = render(
      '| Project | Status |\n| --- | --- |\n| <a href="https://vercel.com/o/r"><img src="https://vercel.com/avatar.png" width="16" height="16" alt="" /></a> avzx | Ready |',
      'forge'
    )

    expect(html).toContain('<table')
    expect(html).toContain('src="https://vercel.com/avatar.png"')
    expect(html).toContain('width="16"')
    expect(html).not.toContain('&lt;img')
  })

  test('drops scripts and event handlers', () => {
    const html = render(
      'hi <script>alert(1)</script><img src="x.png" onerror="alert(2)"><a href="javascript:alert(3)">x</a>'
    )

    expect(html).not.toContain('<script')
    expect(html).not.toContain('onerror')
    expect(html).not.toContain('javascript:')
  })

  test('forge text keeps single line breaks; agent text joins them', () => {
    expect(render('one\ntwo', 'forge')).toContain('<br/>')
    expect(render('one\ntwo')).not.toContain('<br/>')
  })

  test('a fenced block shows its language, with its icon, and its code', () => {
    const html = render('```TS\nconst a = 1 < 2\n```')

    expect(html).toContain('>TypeScript</span>')
    expect(html).toContain('href="#file-tree-builtin-typescript"')
    expect(html).toContain('<code>const a = 1 &lt; 2</code>')
  })

  test('web links carry their site icon; other links do not', () => {
    const web = render('[PR](https://github.com/nxtvoid/avzx/pull/3)')
    const local = render('[section](#usage)')

    expect(web).toContain('class="text-link')
    expect(web).toContain('<svg')
    expect(local).not.toContain('<svg')
  })
})
