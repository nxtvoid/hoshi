/**
 * The centered column the transcript and composer share. When a card floating at the
 * right (`--details-room` wide) would cover it, it moves left only as far as needed,
 * keeping `--details-gutter` from the left edge, and narrows past that.
 */
export const CHAT_LANE =
  'ml-[max(var(--details-gutter,0px),min((100%_-_48rem)/2,100%_-_var(--details-room,0px)_-_48rem))] mr-[max(var(--details-room,0px),(100%_-_48rem)/2)] max-w-3xl'
