import { describe, expect, test } from 'bun:test'
import {
  fileUris,
  imagePaths,
  pastedFiles,
  type Transfer
} from '../../src/lib/pasted-files'

const transfer = (
  data: Record<string, string>,
  files: File[] = [],
  items: { kind: string; type: string; getAsFile: () => File | null }[] = []
): Transfer => ({
  files: files as unknown as FileList,
  items: items as unknown as DataTransferItemList,
  getData: (format) => data[format] ?? ''
})

const png = new File(['png'], 'shot.png', { type: 'image/png' })

describe('imagePaths', () => {
  test('takes paths and file URIs of images, one per line', () => {
    expect(imagePaths('/home/nxtvoid/Pictures/screenshot.png')).toEqual([
      '/home/nxtvoid/Pictures/screenshot.png'
    ])
    expect(
      imagePaths(
        '# copied\nfile:///home/nxtvoid/My%20photos/a.JPG\r\nfile:///C:/x/b.webp'
      )
    ).toEqual(['/home/nxtvoid/My photos/a.JPG', 'C:/x/b.webp'])
    expect(imagePaths('C:\\Users\\nxtvoid\\shot.png')).toEqual([
      'C:\\Users\\nxtvoid\\shot.png'
    ])
  })

  test('leaves text that is not only image paths', () => {
    expect(imagePaths('see /home/nxtvoid/a.png')).toBeNull()
    expect(
      imagePaths('/home/nxtvoid/a.png\n/home/nxtvoid/notes.txt')
    ).toBeNull()
    expect(imagePaths('relative/a.png')).toBeNull()
    expect(imagePaths('')).toBeNull()
  })
})

describe('fileUris', () => {
  test('takes a list of file URIs of any type', () => {
    expect(
      fileUris(
        ['file:///home/nxtvoid/notes.txt', 'file:///C:/x/report%20Q3.pdf'].join(
          '\r\n'
        )
      )
    ).toEqual(['/home/nxtvoid/notes.txt', 'C:/x/report Q3.pdf'])
    expect(fileUris('https://example.com/a.pdf')).toBeNull()
    expect(fileUris('')).toBeNull()
  })
})

describe('pastedFiles', () => {
  test('takes image files, or image items when the files are empty', () => {
    expect(pastedFiles(transfer({}, [png]), { orClipboard: true })).toEqual({
      kind: 'files',
      files: [png]
    })
    const item = { kind: 'file', type: 'image/png', getAsFile: () => png }
    expect(
      pastedFiles(transfer({}, [], [item]), { orClipboard: true })
    ).toEqual({ kind: 'files', files: [png] })
  })

  test('takes the paths of copied image files', () => {
    const copied = transfer({
      'text/plain': '/home/nxtvoid/Pictures/screenshot.png',
      'text/uri-list': 'file:///home/nxtvoid/Pictures/screenshot.png'
    })
    expect(pastedFiles(copied, { orClipboard: true })).toEqual({
      kind: 'paths',
      paths: ['/home/nxtvoid/Pictures/screenshot.png'],
      text: '/home/nxtvoid/Pictures/screenshot.png'
    })
  })

  test('looks at the system clipboard only for a paste without text', () => {
    const image = transfer({ 'text/html': '<img src="cid:1">' })
    expect(pastedFiles(image, { orClipboard: true })).toEqual({
      kind: 'clipboard'
    })
    expect(pastedFiles(image, { orClipboard: false })).toBeNull()
    expect(
      pastedFiles(transfer({ 'text/plain': 'hola' }), { orClipboard: true })
    ).toBeNull()
    expect(
      pastedFiles(transfer({ 'text/html': '<b>hola</b>' }), {
        orClipboard: true
      })
    ).toBeNull()
  })

  test('takes other files only when no text comes with them', () => {
    const pdf = new File(['%PDF-'], 'report.pdf', { type: 'application/pdf' })
    expect(pastedFiles(transfer({}, [pdf]), { orClipboard: true })).toEqual({
      kind: 'files',
      files: [pdf]
    })
    expect(
      pastedFiles(transfer({ 'text/plain': 'a table' }, [pdf]), {
        orClipboard: true
      })
    ).toBeNull()
    expect(
      pastedFiles(transfer({ 'text/plain': 'a chart' }, [png, pdf]), {
        orClipboard: true
      })
    ).toEqual({ kind: 'files', files: [png, pdf] })
  })

  test('takes the file URIs a file manager copied, of any type', () => {
    const copied = transfer({
      'text/plain': '/home/nxtvoid/notes.txt',
      'text/uri-list': 'file:///home/nxtvoid/notes.txt'
    })
    expect(pastedFiles(copied, { orClipboard: true })).toEqual({
      kind: 'paths',
      paths: ['/home/nxtvoid/notes.txt'],
      text: '/home/nxtvoid/notes.txt'
    })
    expect(
      pastedFiles(transfer({ 'text/plain': '/home/nxtvoid/notes.txt' }), {
        orClipboard: true
      })
    ).toBeNull()
  })
})
