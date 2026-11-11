type Size = { width: number; height: number }

/**
 * The size a page of its own size shows at inside the panel: whole, and never larger than
 * it is.
 */
export function fitInside(panel: Size, page: Size): Size {
  if (page.width <= 0 || page.height <= 0) return { width: 0, height: 0 }
  const scale = Math.min(
    1,
    panel.width / page.width,
    panel.height / page.height
  )
  return {
    width: Math.max(0, Math.floor(page.width * scale)),
    height: Math.max(0, Math.floor(page.height * scale))
  }
}
