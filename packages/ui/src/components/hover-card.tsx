import { PreviewCard as PreviewCardPrimitive } from '@base-ui/react/preview-card'
import { cn } from 'cn'

function HoverCard<Payload>({
  ...props
}: PreviewCardPrimitive.Root.Props<Payload>) {
  return <PreviewCardPrimitive.Root data-slot='hover-card' {...props} />
}

function HoverCardTrigger<Payload>({
  ...props
}: PreviewCardPrimitive.Trigger.Props<Payload>) {
  return (
    <PreviewCardPrimitive.Trigger data-slot='hover-card-trigger' {...props} />
  )
}

/** Shares one card between many triggers, which pass it their `payload`. */
const createHoverCardHandle = PreviewCardPrimitive.createHandle

function HoverCardContent({
  className,
  positionerClassName,
  side = 'bottom',
  sideOffset = 4,
  align = 'center',
  alignOffset = 4,
  ...props
}: PreviewCardPrimitive.Popup.Props &
  Pick<
    PreviewCardPrimitive.Positioner.Props,
    'align' | 'alignOffset' | 'side' | 'sideOffset'
  > & {
    /** On the element that places the card, to animate it moving between triggers. */
    positionerClassName?: string
  }) {
  return (
    <PreviewCardPrimitive.Portal data-slot='hover-card-portal'>
      <PreviewCardPrimitive.Positioner
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
        className={cn('isolate z-50', positionerClassName)}
      >
        <PreviewCardPrimitive.Popup
          data-slot='hover-card-content'
          className={cn(
            'bg-popover text-popover-foreground ring-foreground/10 data-[side=bottom]:slide-in-from-top-2 data-[side=inline-end]:slide-in-from-left-2 data-[side=inline-start]:slide-in-from-right-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95 data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95 z-50 w-64 origin-(--transform-origin) rounded-lg p-2.5 text-sm shadow-md ring-1 outline-hidden duration-100',
            className
          )}
          {...props}
        />
      </PreviewCardPrimitive.Positioner>
    </PreviewCardPrimitive.Portal>
  )
}

export { HoverCard, HoverCardTrigger, HoverCardContent, createHoverCardHandle }
