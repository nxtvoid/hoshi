import { describe, expect, test } from 'bun:test'
import {
  attachesPastedText,
  attachmentMimeType,
  attachmentProblem,
  classifyAttachment,
  formatAttachmentLimit,
  isPasteAsTextShortcut,
  MAX_ATTACHMENTS,
  MAX_FILE_BYTES,
  PASTED_TEXT_ATTACHMENT_BYTES,
  pastedTextFileName
} from '../../src/lib/attachment-files'

const named = (name: string, type = '') => ({ name, type })

describe('classifyAttachment', () => {
  test('sends the images the app can draw as images', () => {
    expect(classifyAttachment(named('a.png', 'image/png'))).toBe('image')
    expect(classifyAttachment(named('a.bmp', 'image/bmp'))).toBe('image')
    expect(classifyAttachment(named('shot.jpg'))).toBe('image')
    expect(
      classifyAttachment(named('shot.webp', 'application/octet-stream'))
    ).toBe('image')
  })

  test('sends vector and raw images, and everything else, as files', () => {
    expect(classifyAttachment(named('logo.svg', 'image/svg+xml'))).toBe('file')
    expect(classifyAttachment(named('raw.dng', 'image/x-adobe-dng'))).toBe(
      'file'
    )
    expect(classifyAttachment(named('report.pdf', 'application/pdf'))).toBe(
      'file'
    )
    expect(classifyAttachment(named('notes'))).toBe('file')
  })

  test('refuses image types neither the app nor the agent can show', () => {
    expect(classifyAttachment(named('photo.heic', 'image/heic'))).toBe(
      'unsupportedImage'
    )
  })
})

describe('attachmentMimeType', () => {
  test('keeps a file its type, or reads it from its name', () => {
    expect(attachmentMimeType(named('a.pdf', 'application/pdf'))).toBe(
      'application/pdf'
    )
    expect(attachmentMimeType(named('a.pdf'))).toBe('application/pdf')
    expect(attachmentMimeType(named('logo.svg', 'text/xml'))).toBe(
      'image/svg+xml'
    )
    expect(attachmentMimeType(named('blob'))).toBe('application/octet-stream')
  })
})

describe('attachmentProblem', () => {
  const pdf = { name: 'a.pdf', type: 'application/pdf', size: 10 }

  test('lets a file in within the limits', () => {
    expect(attachmentProblem(pdf, 0)).toBeNull()
  })

  test('says why a file stays out', () => {
    expect(attachmentProblem(pdf, MAX_ATTACHMENTS)).toEqual({
      kind: 'tooMany',
      max: MAX_ATTACHMENTS
    })
    expect(attachmentProblem({ ...pdf, size: 0 }, 0)).toEqual({
      kind: 'empty',
      name: 'a.pdf'
    })
    expect(attachmentProblem({ ...pdf, size: MAX_FILE_BYTES + 1 }, 0)).toEqual({
      kind: 'tooLarge',
      name: 'a.pdf',
      max: MAX_FILE_BYTES
    })
    expect(
      attachmentProblem({ name: 'a.heic', type: 'image/heic', size: 1 }, 0)
    ).toEqual({ kind: 'unsupportedImage', name: 'a.heic' })
  })
})

describe('pasted text', () => {
  test('attaches a paste as large as the threshold, counted in UTF-8', () => {
    expect(attachesPastedText('a'.repeat(PASTED_TEXT_ATTACHMENT_BYTES))).toBe(
      true
    )
    expect(
      attachesPastedText('a'.repeat(PASTED_TEXT_ATTACHMENT_BYTES - 1))
    ).toBe(false)
    expect(
      attachesPastedText('é'.repeat(PASTED_TEXT_ATTACHMENT_BYTES / 2))
    ).toBe(true)
  })

  test('names each pasted text file after the ones the draft has', () => {
    expect(pastedTextFileName([])).toBe('pasted-text.txt')
    expect(pastedTextFileName(['Pasted-Text.txt', 'pasted-text-2.txt'])).toBe(
      'pasted-text-3.txt'
    )
  })

  test('knows the paste-as-text shortcut on each system', () => {
    const keys = {
      key: 'V',
      code: 'KeyV',
      shiftKey: true,
      altKey: false,
      ctrlKey: true,
      metaKey: false
    }
    expect(isPasteAsTextShortcut(keys, false)).toBe(true)
    expect(isPasteAsTextShortcut(keys, true)).toBe(false)
    expect(
      isPasteAsTextShortcut({ ...keys, ctrlKey: false, metaKey: true }, true)
    ).toBe(true)
    expect(isPasteAsTextShortcut({ ...keys, key: 'м' }, false)).toBe(true)
    expect(isPasteAsTextShortcut({ ...keys, shiftKey: false }, false)).toBe(
      false
    )
  })
})

describe('formatAttachmentLimit', () => {
  test('writes a limit in the unit it was set in', () => {
    expect(formatAttachmentLimit(50 * 1024 * 1024)).toBe('50 MB')
    expect(formatAttachmentLimit(512 * 1024)).toBe('512 KB')
    expect(formatAttachmentLimit(1)).toBe('1 byte')
    expect(formatAttachmentLimit(1500)).toBe('1500 bytes')
  })
})
