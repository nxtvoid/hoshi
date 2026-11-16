//! Which tooltip shows and when, apart from GPUI: Base UI's hover, focus and dismiss rules with
//! its provider's delay group. Time comes in as arguments; timers go out as effects.

use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub delay: Duration,
    pub close_delay: Duration,
    /// How long after a tooltip closes the next one still opens at once.
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shown<K> {
    pub key: K,
    /// Opened at once from another tooltip of the group: no enter animation.
    pub instant: bool,
    pub by_focus: bool,
    pub generation: u64,
}

/// What a transition asks of the caller: redraw the tooltips that appeared or went away, and
/// call [`Machine::fire`] with the generation once the timer runs out.
#[derive(Debug, PartialEq, Eq)]
pub struct Effect<K> {
    pub shown: Option<K>,
    pub hidden: Option<K>,
    pub timer: Option<(Duration, u64)>,
}

impl<K> Default for Effect<K> {
    fn default() -> Self {
        Self {
            shown: None,
            hidden: None,
            timer: None,
        }
    }
}

#[derive(Debug)]
struct Timer<K> {
    key: K,
    generation: u64,
}

#[derive(Debug)]
pub struct Machine<K> {
    shown: Option<Shown<K>>,
    opening: Option<Timer<K>>,
    closing: Option<Timer<K>>,
    /// The tooltip the group remembers, and when it closed.
    last: Option<K>,
    closed_at: Option<Instant>,
    generation: u64,
}

impl<K> Default for Machine<K> {
    fn default() -> Self {
        Self {
            shown: None,
            opening: None,
            closing: None,
            last: None,
            closed_at: None,
            generation: 0,
        }
    }
}

impl<K: Clone + PartialEq> Machine<K> {
    pub fn shown(&self) -> Option<&Shown<K>> {
        self.shown.as_ref()
    }

    pub fn is_shown(&self, key: &K) -> bool {
        self.shown.as_ref().is_some_and(|shown| shown.key == *key)
    }

    /// Whether the tooltip shows, waits to open or waits to close.
    pub fn involves(&self, key: &K) -> bool {
        self.is_shown(key)
            || self.opening.as_ref().is_some_and(|timer| timer.key == *key)
            || self.closing.as_ref().is_some_and(|timer| timer.key == *key)
    }

    /// The pointer entered the trigger.
    pub fn hover(&mut self, key: K, now: Instant, timing: Timing) -> Effect<K> {
        if self.closing.as_ref().is_some_and(|timer| timer.key == key) {
            self.closing = None;
            return Effect::default();
        }
        if self.involves(&key) {
            return Effect::default();
        }
        if self.active(now, timing.timeout).is_some() || timing.delay.is_zero() {
            return self.show(key, false, now, timing.timeout);
        }
        let generation = self.next();
        self.opening = Some(Timer { key, generation });
        Effect {
            timer: Some((timing.delay, generation)),
            ..Effect::default()
        }
    }

    /// The pointer left the trigger.
    pub fn leave(&mut self, key: &K, now: Instant, timing: Timing) -> Effect<K> {
        if self.opening.as_ref().is_some_and(|timer| timer.key == *key) {
            self.opening = None;
        }
        let hovered_open = self
            .shown
            .as_ref()
            .is_some_and(|shown| shown.key == *key && !shown.by_focus);
        if !hovered_open || self.closing.is_some() {
            return Effect::default();
        }
        if timing.close_delay.is_zero() {
            return self.hide(now);
        }
        let generation = self.next();
        self.closing = Some(Timer {
            key: key.clone(),
            generation,
        });
        Effect {
            timer: Some((timing.close_delay, generation)),
            ..Effect::default()
        }
    }

    /// Keyboard focus reached the trigger: Base UI opens at once.
    pub fn focus(&mut self, key: K, now: Instant, timing: Timing) -> Effect<K> {
        if self.is_shown(&key) {
            return Effect::default();
        }
        self.show(key, true, now, timing.timeout)
    }

    pub fn blur(&mut self, key: &K, now: Instant) -> Effect<K> {
        match &self.shown {
            Some(shown) if shown.key == *key && shown.by_focus => self.hide(now),
            _ => Effect::default(),
        }
    }

    /// A timer ran out; stale generations do nothing.
    pub fn fire(&mut self, generation: u64, now: Instant, timeout: Duration) -> Effect<K> {
        if let Some(timer) = self.opening.take_if(|timer| timer.generation == generation) {
            return self.show(timer.key, false, now, timeout);
        }
        if self
            .closing
            .take_if(|timer| timer.generation == generation)
            .is_some()
        {
            return self.hide(now);
        }
        Effect::default()
    }

    /// A press, Escape, or the window going inactive: closes without waiting. The next trigger
    /// still opens at once for a moment, as after any close.
    pub fn dismiss(&mut self, now: Instant) -> Effect<K> {
        self.opening = None;
        self.closing = None;
        self.hide(now)
    }

    /// Scrolling closes too, and the next trigger waits its full delay again.
    pub fn scroll(&mut self, now: Instant) -> Effect<K> {
        let effect = self.dismiss(now);
        self.last = None;
        self.closed_at = None;
        effect
    }

    /// The trigger is gone, or its tooltip was disabled.
    pub fn unmount(&mut self, key: &K) -> Effect<K> {
        if self.opening.as_ref().is_some_and(|timer| timer.key == *key) {
            self.opening = None;
        }
        if self.closing.as_ref().is_some_and(|timer| timer.key == *key) {
            self.closing = None;
        }
        if self.last.as_ref() == Some(key) {
            self.last = None;
            self.closed_at = None;
        }
        let hidden = self
            .shown
            .take_if(|shown| shown.key == *key)
            .map(|shown| shown.key);
        Effect {
            hidden,
            ..Effect::default()
        }
    }

    /// The group's tooltip while it shows or for `timeout` after it closed: the next trigger
    /// skips its delay.
    fn active(&self, now: Instant, timeout: Duration) -> Option<&K> {
        if let Some(shown) = &self.shown {
            return Some(&shown.key);
        }
        let closed_at = self.closed_at?;
        if now.saturating_duration_since(closed_at) < timeout {
            self.last.as_ref()
        } else {
            None
        }
    }

    fn show(&mut self, key: K, by_focus: bool, now: Instant, timeout: Duration) -> Effect<K> {
        let instant = self
            .active(now, timeout)
            .is_some_and(|active| *active != key);
        let hidden = self.shown.take().map(|shown| shown.key);
        self.opening = None;
        self.closing = None;
        self.last = Some(key.clone());
        self.closed_at = None;
        let generation = self.next();
        self.shown = Some(Shown {
            key: key.clone(),
            instant,
            by_focus,
            generation,
        });
        Effect {
            shown: Some(key),
            hidden,
            timer: None,
        }
    }

    fn hide(&mut self, now: Instant) -> Effect<K> {
        let Some(shown) = self.shown.take() else {
            return Effect::default();
        };
        self.closing = None;
        self.closed_at = Some(now);
        Effect {
            hidden: Some(shown.key),
            ..Effect::default()
        }
    }

    fn next(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }
}

#[cfg(test)]
mod tests {
    use super::{Effect, Machine, Timing};
    use std::time::{Duration, Instant};

    const TIMING: Timing = Timing {
        delay: Duration::from_millis(600),
        close_delay: Duration::ZERO,
        timeout: Duration::from_millis(400),
    };

    fn ms(millis: u64) -> Duration {
        Duration::from_millis(millis)
    }

    fn opened(machine: &mut Machine<&'static str>, key: &'static str, now: Instant) {
        let effect = machine.hover(key, now, TIMING);
        let (delay, generation) = effect.timer.expect("waits for the delay");
        assert_eq!(delay, TIMING.delay);
        assert_eq!(
            machine.fire(generation, now + delay, TIMING.timeout).shown,
            Some(key)
        );
    }

    #[test]
    fn a_tooltip_opens_once_the_delay_runs_out() {
        let mut machine = Machine::default();
        let now = Instant::now();
        let effect = machine.hover("a", now, TIMING);
        assert_eq!(effect.timer.map(|(delay, _)| delay), Some(ms(600)));
        assert!(!machine.is_shown(&"a"));
        let generation = effect.timer.unwrap().1;
        let effect = machine.fire(generation, now + ms(600), TIMING.timeout);
        assert_eq!(effect.shown, Some("a"));
        let shown = machine.shown().unwrap();
        assert!(!shown.instant && !shown.by_focus);
    }

    #[test]
    fn leaving_before_the_delay_cancels_the_open() {
        let mut machine = Machine::default();
        let now = Instant::now();
        let (_, generation) = machine.hover("a", now, TIMING).timer.unwrap();
        machine.leave(&"a", now + ms(100), TIMING);
        assert_eq!(
            machine.fire(generation, now + ms(600), TIMING.timeout),
            Effect::default()
        );
        assert!(machine.shown().is_none());
    }

    #[test]
    fn the_next_trigger_opens_at_once_while_one_shows() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        let effect = machine.hover("b", now + ms(700), TIMING);
        assert_eq!((effect.shown, effect.hidden), (Some("b"), Some("a")));
        assert!(machine.shown().unwrap().instant);
    }

    #[test]
    fn the_group_opens_at_once_only_within_its_timeout() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        let left = now + ms(1_000);
        assert_eq!(machine.leave(&"a", left, TIMING).hidden, Some("a"));

        assert_eq!(machine.hover("b", left + ms(300), TIMING).shown, Some("b"));
        assert!(machine.shown().unwrap().instant);
        machine.leave(&"b", left + ms(400), TIMING);

        let late = machine.hover("c", left + ms(800) + ms(1), TIMING);
        assert!(late.timer.is_some() && late.shown.is_none());
    }

    #[test]
    fn the_same_trigger_reopens_at_once_but_animates() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        machine.leave(&"a", now + ms(700), TIMING);
        assert_eq!(machine.hover("a", now + ms(800), TIMING).shown, Some("a"));
        assert!(!machine.shown().unwrap().instant);
    }

    #[test]
    fn a_close_delay_waits_and_reentering_keeps_it_open() {
        let timing = Timing {
            close_delay: ms(200),
            ..TIMING
        };
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        let (delay, generation) = machine.leave(&"a", now, timing).timer.unwrap();
        assert_eq!(delay, ms(200));
        assert!(machine.is_shown(&"a"));
        assert_eq!(machine.hover("a", now + ms(100), timing), Effect::default());
        assert_eq!(
            machine.fire(generation, now + ms(200), timing.timeout),
            Effect::default()
        );
        assert!(machine.is_shown(&"a"));

        let (_, generation) = machine.leave(&"a", now + ms(300), timing).timer.unwrap();
        let effect = machine.fire(generation, now + ms(500), timing.timeout);
        assert_eq!(effect.hidden, Some("a"));
    }

    #[test]
    fn a_dismissal_closes_and_cancels_the_pending_open() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        assert_eq!(machine.dismiss(now + ms(700)).hidden, Some("a"));

        let mut machine = Machine::default();
        let (_, generation) = machine.hover("a", now, TIMING).timer.unwrap();
        machine.dismiss(now + ms(100));
        assert!(
            machine
                .fire(generation, now + ms(600), TIMING.timeout)
                .shown
                .is_none()
        );
    }

    #[test]
    fn scrolling_ends_the_group_window() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        machine.scroll(now + ms(700));
        assert!(machine.hover("b", now + ms(750), TIMING).timer.is_some());
    }

    #[test]
    fn focus_opens_at_once_and_stays_until_blur() {
        let mut machine = Machine::default();
        let now = Instant::now();
        assert_eq!(machine.focus("a", now, TIMING).shown, Some("a"));
        assert!(machine.shown().unwrap().by_focus);
        machine.hover("a", now, TIMING);
        assert_eq!(machine.leave(&"a", now, TIMING), Effect::default());
        assert_eq!(machine.blur(&"a", now).hidden, Some("a"));
    }

    #[test]
    fn an_unmounted_trigger_closes_and_resets_the_group() {
        let mut machine = Machine::default();
        let now = Instant::now();
        opened(&mut machine, "a", now);
        assert_eq!(machine.unmount(&"a").hidden, Some("a"));
        assert!(machine.hover("b", now + ms(700), TIMING).timer.is_some());
    }

    #[test]
    fn a_zero_delay_opens_on_hover() {
        let mut machine = Machine::default();
        let timing = Timing {
            delay: Duration::ZERO,
            ..TIMING
        };
        assert_eq!(machine.hover("a", Instant::now(), timing).shown, Some("a"));
    }
}
