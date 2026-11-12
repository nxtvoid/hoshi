/** A frame of a thread's browser page as the engine sends it, before it is shown. */
export type RawBrowserFrame = {
  threadId: string
  /** The page's size in CSS pixels, which clicks on the frame map back to. */
  width: number
  height: number
  jpeg: Uint8Array<ArrayBuffer>
}

/**
 * Reads the bytes `BrowserFrame::to_bytes` writes, little-endian: the thread id's length
 * as a `u16` and its UTF-8, the width and the height as `u32`, then the JPEG.
 */
export function readBrowserFrame(buffer: ArrayBuffer): RawBrowserFrame {
  const view = new DataView(buffer)
  const idLength = view.getUint16(0, true)
  const idEnd = 2 + idLength
  return {
    threadId: new TextDecoder().decode(new Uint8Array(buffer, 2, idLength)),
    width: view.getUint32(idEnd, true),
    height: view.getUint32(idEnd + 4, true),
    jpeg: new Uint8Array(buffer, idEnd + 8)
  }
}
