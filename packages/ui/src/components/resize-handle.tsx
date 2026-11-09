import { useCallback, useEffect, useRef } from 'react'
import { cn } from 'cn'

const KEYBOARD_STEP = 16

const EDGE_CLASS = {
  left: '-left-0.5 inset-y-0 w-1 cursor-col-resize',
  right: '-right-0.5 inset-y-0 w-1 cursor-col-resize',
  top: '-top-0.5 inset-x-0 h-1 cursor-row-resize',
  bottom: '-bottom-0.5 inset-x-0 h-1 cursor-row-resize'
} as const

type ResizeHandleProps = {
  /** Current size in pixels: the width on a side edge, the height on a top or bottom one. */
  value: number
  min: number
  max: number
  /** Restored on double click. */
  defaultValue: number
  onChange: (value: number) => void
  /** The edge of the resized element the handle sits on. */
  edge: 'left' | 'right' | 'top' | 'bottom'
  label: string
  className?: string
}

/** A draggable, focusable splitter on one edge of a panel. */
function ResizeHandle({
  value,
  min,
  max,
  defaultValue,
  onChange,
  edge,
  label,
  className
}: ResizeHandleProps) {
  const drag = useRef<{ start: number; startValue: number } | null>(null)
  const vertical = edge === 'top' || edge === 'bottom'

  // While dragging, nothing else on the page may start a text selection; WebKit still needs
  // the prefixed property.
  const end = useCallback(() => {
    if (!drag.current) return
    drag.current = null
    const style = document.body.style
    style.userSelect = ''
    style.webkitUserSelect = ''
    style.cursor = ''
  }, [])
  useEffect(() => {
    window.addEventListener('blur', end)
    return () => {
      window.removeEventListener('blur', end)
      end()
    }
  }, [end])
  const direction = edge === 'right' || edge === 'bottom' ? 1 : -1
  const [less, more] = vertical
    ? ['ArrowUp', 'ArrowDown']
    : ['ArrowLeft', 'ArrowRight']

  return (
    // biome-ignore lint/a11y/useSemanticElements: a focusable splitter has no native element
    <div
      role='separator'
      aria-orientation={vertical ? 'horizontal' : 'vertical'}
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={value}
      tabIndex={0}
      onPointerDown={(event) => {
        if (event.button !== 0) return
        event.preventDefault()
        window.getSelection()?.removeAllRanges()
        event.currentTarget.setPointerCapture(event.pointerId)
        const style = document.body.style
        style.userSelect = 'none'
        style.webkitUserSelect = 'none'
        style.cursor = vertical ? 'row-resize' : 'col-resize'
        drag.current = {
          start: vertical ? event.clientY : event.clientX,
          startValue: value
        }
      }}
      onMouseDown={(event) => event.preventDefault()}
      onPointerMove={(event) => {
        if (!drag.current) return
        const moved =
          (vertical ? event.clientY : event.clientX) - drag.current.start
        onChange(drag.current.startValue + moved * direction)
      }}
      onPointerUp={end}
      onPointerCancel={end}
      onLostPointerCapture={end}
      onDoubleClick={() => onChange(defaultValue)}
      onKeyDown={(event) => {
        if (event.key === less) onChange(value - KEYBOARD_STEP * direction)
        else if (event.key === more) {
          onChange(value + KEYBOARD_STEP * direction)
        } else return
        event.preventDefault()
      }}
      className={cn(
        'hover:bg-ring/40 focus-visible:bg-ring/60 active:bg-ring/60 absolute z-10 touch-none outline-none select-none',
        EDGE_CLASS[edge],
        className
      )}
    />
  )
}

export { ResizeHandle }
