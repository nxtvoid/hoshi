import { describe, expect, test } from 'bun:test'
import {
  imagePaths,
  pastedImages,
  type Transfer
} from '../../src/lib/pasted-images'

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

describe('pastedImages', () => {
  test('takes image files, or image items when the files are empty', () => {
    expect(pastedImages(transfer({}, [png]), { orClipboard: true })).toEqual({
      kind: 'files',
      files: [png]
    })
    const item = { kind: 'file', type: 'image/png', getAsFile: () => png }
    expect(
      pastedImages(transfer({}, [], [item]), { orClipboard: true })
    ).toEqual({ kind: 'files', files: [png] })
  })

  test('takes the paths of copied image files', () => {
    const copied = transfer({
      'text/plain': '/home/nxtvoid/Pictures/screenshot.png',
      'text/uri-list': 'file:///home/nxtvoid/Pictures/screenshot.png'
    })
    expect(pastedImages(copied, { orClipboard: true })).toEqual({
      kind: 'paths',
      paths: ['/home/nxtvoid/Pictures/screenshot.png'],
      text: '/home/nxtvoid/Pictures/screenshot.png'
    })
  })

  test('looks at the system clipboard only for a paste without text', () => {
    const image = transfer({ 'text/html': '<img src="cid:1">' })
    expect(pastedImages(image, { orClipboard: true })).toEqual({
      kind: 'clipboard'
    })
    expect(pastedImages(image, { orClipboard: false })).toBeNull()
    expect(
      pastedImages(transfer({ 'text/plain': 'hola' }), { orClipboard: true })
    ).toBeNull()
    expect(
      pastedImages(transfer({ 'text/html': '<b>hola</b>' }), {
        orClipboard: true
      })
    ).toBeNull()
  })
})
