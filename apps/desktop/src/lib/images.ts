/** What every provider reads; anything else is redrawn as PNG or JPEG. */
const SUPPORTED_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp']
/** Providers refuse larger images. */
const MAX_BYTES = 5 * 1024 * 1024
/** Larger images are scaled down before anything else. */
const MAX_EDGE = 4096

export type PreparedImage = { name: string; mimeType: string; data: string }

/** The image as a provider takes it, its bytes in base64. */
export async function prepareImage(file: File): Promise<PreparedImage> {
  if (SUPPORTED_TYPES.includes(file.type) && file.size <= MAX_BYTES) {
    return { name: file.name, mimeType: file.type, data: await base64(file) }
  }
  const bitmap = await createImageBitmap(file)
  try {
    let scale = Math.min(1, MAX_EDGE / Math.max(bitmap.width, bitmap.height))
    const mimeType =
      file.type === 'image/png' || file.size <= MAX_BYTES
        ? 'image/png'
        : 'image/jpeg'
    for (let attempt = 0; attempt < 6; attempt++) {
      const blob = await redraw(bitmap, scale, mimeType)
      if (blob.size <= MAX_BYTES) {
        return { name: file.name, mimeType, data: await base64(blob) }
      }
      scale *= 0.7
    }
    throw new Error('image too large')
  } finally {
    bitmap.close()
  }
}

async function redraw(bitmap: ImageBitmap, scale: number, type: string) {
  const width = Math.max(1, Math.round(bitmap.width * scale))
  const height = Math.max(1, Math.round(bitmap.height * scale))
  const canvas = new OffscreenCanvas(width, height)
  canvas.getContext('2d')?.drawImage(bitmap, 0, 0, width, height)
  return canvas.convertToBlob({ type, quality: 0.9 })
}

function base64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const url = String(reader.result)
      resolve(url.slice(url.indexOf(',') + 1))
    }
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(blob)
  })
}

/** Images in a paste, drop or file input; other files are left alone. */
export function imageFiles(source: { files: FileList | null } | null): File[] {
  return Array.from(source?.files ?? []).filter((file) =>
    file.type.startsWith('image/')
  )
}
