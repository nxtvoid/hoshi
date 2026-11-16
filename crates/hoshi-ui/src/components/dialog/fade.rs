use std::time::{Duration, Instant};

/// `duration-150` in the web dialog.
pub(crate) const DURATION: Duration = Duration::from_millis(150);

/// A 0..1 visibility that eases toward its target and can turn around midway.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fade {
    from: f32,
    to: f32,
    start: Instant,
}

impl Fade {
    pub fn opening(now: Instant) -> Self {
        Self {
            from: 0.,
            to: 1.,
            start: now,
        }
    }

    pub fn value(&self, now: Instant, duration: Duration) -> f32 {
        let progress = self.progress(now, duration);
        self.from + (self.to - self.from) * ease_out(progress)
    }

    pub fn is_settled(&self, now: Instant, duration: Duration) -> bool {
        self.progress(now, duration) >= 1.
    }

    pub fn remaining(&self, now: Instant, duration: Duration) -> Duration {
        let span = self.span(duration);
        span.saturating_sub(now.saturating_duration_since(self.start))
    }

    pub fn toward(&mut self, to: f32, now: Instant, duration: Duration) {
        let from = self.value(now, duration);
        *self = Self {
            from,
            to,
            start: now,
        };
    }

    fn span(&self, duration: Duration) -> Duration {
        duration.mul_f32((self.to - self.from).abs())
    }

    fn progress(&self, now: Instant, duration: Duration) -> f32 {
        let span = self.span(duration);
        if span.is_zero() {
            return 1.;
        }
        let elapsed = now.saturating_duration_since(self.start);
        (elapsed.as_secs_f32() / span.as_secs_f32()).clamp(0., 1.)
    }
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t).powi(3)
}

#[cfg(test)]
mod tests {
    use super::{DURATION, Fade};
    use std::time::{Duration, Instant};

    #[test]
    fn opens_from_hidden_to_shown() {
        let start = Instant::now();
        let fade = Fade::opening(start);
        assert_eq!(fade.value(start, DURATION), 0.);
        let halfway = fade.value(start + DURATION / 2, DURATION);
        assert!(halfway > 0.5 && halfway < 1., "{halfway}");
        assert_eq!(fade.value(start + DURATION, DURATION), 1.);
        assert!(fade.is_settled(start + DURATION, DURATION));
        assert!(!fade.is_settled(start + DURATION / 2, DURATION));
    }

    #[test]
    fn turning_around_continues_from_the_current_value() {
        let start = Instant::now();
        let mut fade = Fade::opening(start);
        let turn = start + DURATION / 3;
        let at_turn = fade.value(turn, DURATION);
        fade.toward(0., turn, DURATION);
        assert_eq!(fade.value(turn, DURATION), at_turn);
        assert!(fade.remaining(turn, DURATION) < DURATION);
        let end = turn + fade.remaining(turn, DURATION);
        assert_eq!(fade.value(end, DURATION), 0.);
    }

    #[test]
    fn no_duration_settles_at_once() {
        let start = Instant::now();
        let mut fade = Fade::opening(start);
        assert_eq!(fade.value(start, Duration::ZERO), 1.);
        fade.toward(0., start, Duration::ZERO);
        assert_eq!(fade.value(start, Duration::ZERO), 0.);
        assert_eq!(fade.remaining(start, Duration::ZERO), Duration::ZERO);
    }
}
