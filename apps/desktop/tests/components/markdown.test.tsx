import { describe, expect, test } from 'bun:test'
import { renderToStaticMarkup } from 'react-dom/server'
import { Markdown } from '../../src/components/markdown/Markdown'
import { knownMentions } from '../../src/lib/prompt-segments'

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

  test('a fenced block without a language shows it as plain text', () => {
    const html = render('```\nhola\n```')

    expect(html).toContain('>Plain text</span>')
    expect(html).toContain('<code>hola</code>')
  })

  test('web links carry their site icon; other links do not', () => {
    const web = render('[PR](https://github.com/nxtvoid/avzx/pull/3)')
    const local = render('[section](#usage)')

    expect(web).toContain('class="text-link')
    expect(web).toContain('<svg')
    expect(local).not.toContain('<svg')
  })

  test('an agent links files by their path; forge text cannot', () => {
    const agent = render(
      '[the log](C:/work/run.log) and [main](/repo/src/main.rs:12)'
    )
    const forge = render('[the log](C:/work/run.log)', 'forge')

    expect(agent.match(/<button/g)?.length).toBe(2)
    expect(agent).toContain('the log')
    expect(forge).not.toContain('<button')
    expect(forge).not.toContain('C:/work')
  })

  test('a local image waits for the engine to read it', () => {
    expect(render('![shot](C:/work/shot.png)')).not.toContain('<img')
    expect(render('![shot](https://example.com/shot.png)')).toContain(
      'src="https://example.com/shot.png"'
    )
  })

  test('a file a message sent shows as a chip that opens it', () => {
    const report = {
      type: 'file' as const,
      id: 'f1',
      name: 'Q3 report.pdf',
      mimeType: 'application/pdf',
      size: 2048
    }
    const html = renderToStaticMarkup(
      <Markdown
        text='summarize [File #1], not [File #2]'
        source='forge'
        mentions={knownMentions([report])}
      />
    )

    expect(html.match(/<button/g)?.length).toBe(1)
    expect(html).toContain('aria-label="Open Q3 report.pdf"')
    expect(html).toContain('[File #2]')
    expect(html).not.toContain('[File #1]')
  })
})
