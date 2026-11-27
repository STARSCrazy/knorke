//! Linear fades, advanced one frame at a time.

#[derive(Clone, Copy, Debug)]
pub(crate) struct Ramp {
    value: f32,
    target: f32,
    step: f32,
    left: u32,
}

impl Ramp {
    pub(crate) fn at(value: f32) -> Self {
        Self {
            value,
            target: value,
            step: 0.0,
            left: 0,
        }
    }

    pub(crate) fn value(&self) -> f32 {
        self.value
    }

    pub(crate) fn to(&mut self, target: f32, frames: u32) {
        self.target = target;
        self.left = frames;
        if frames == 0 {
            self.value = target;
        } else {
            self.step = (target - self.value) / frames as f32;
        }
    }

    #[inline]
    pub(crate) fn next(&mut self) -> f32 {
        if self.left > 0 {
            self.left -= 1;
            // Land exactly on the target, whatever the rounding on the way.
            self.value = if self.left == 0 {
                self.target
            } else {
                self.value + self.step
            };
        }
        self.value
    }

    pub(crate) fn skip(&mut self, frames: u32) {
        if frames >= self.left {
            self.to(self.target, 0);
        } else {
            self.left -= frames;
            self.value += self.step * frames as f32;
        }
    }

    /// Done fading and resting exactly on `value`.
    pub(crate) fn settled_at(&self, value: f32) -> bool {
        self.left == 0 && self.value == value
    }

    pub(crate) fn silent(&self) -> bool {
        self.settled_at(0.0)
    }
}
