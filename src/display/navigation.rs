use embedded_graphics::prelude::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Previous,
    Next,
}

/// Logical touch handling, independent of the physical touch controller.
/// Pass one press, zero or more moves, then a release in calibrated pixels.
pub struct Navigation {
    index: usize,
    count: usize,
    interval_ms: Option<u64>,
    due_ms: u64,
    pressed: Option<Button>,
}
impl Navigation {
    pub fn new(count: usize, rotation_secs: Option<u32>, now_ms: u64) -> Self {
        let interval_ms = rotation_secs
            .filter(|n| *n > 0)
            .map(|n| u64::from(n) * 1000);
        Self {
            index: 0,
            count: count.max(1),
            interval_ms,
            due_ms: now_ms.saturating_add(interval_ms.unwrap_or(u64::MAX)),
            pressed: None,
        }
    }
    pub fn index(&self) -> usize {
        self.index
    }
    /// Resume a full interval after leaving system controls.
    pub fn restart_timer(&mut self, now_ms: u64) {
        self.due_ms = now_ms.saturating_add(self.interval_ms.unwrap_or(u64::MAX));
    }
    pub fn pressed(&self) -> Option<Button> {
        self.pressed
    }
    fn hit(&self, p: Point) -> Option<Button> {
        if self.count < 2 || !(200..240).contains(&p.y) {
            return None;
        }
        match p.x {
            0..48 => Some(Button::Previous),
            272..320 => Some(Button::Next),
            _ => None,
        }
    }
    pub fn press(&mut self, p: Point) {
        self.pressed = self.hit(p);
    }
    pub fn move_to(&mut self, p: Point) {
        // Four pixels of movement tolerance absorb resistive edge jitter. The
        // release still has to land inside the original button to activate it.
        if let Some(button) = self.pressed {
            let inside_x = match button {
                Button::Previous => (-4..52).contains(&p.x),
                Button::Next => (268..324).contains(&p.x),
            };
            if !inside_x || !(196..244).contains(&p.y) {
                self.pressed = None;
            }
        }
    }
    pub fn release(&mut self, p: Point, now_ms: u64) -> bool {
        let pressed = self.pressed.take();
        if pressed.is_some() && self.hit(p) == pressed {
            self.index = match pressed {
                Some(Button::Previous) => (self.index + self.count - 1) % self.count,
                _ => (self.index + 1) % self.count,
            };
            self.due_ms = now_ms.saturating_add(self.interval_ms.unwrap_or(u64::MAX));
            true
        } else {
            false
        }
    }
    /// One transition when due; delayed frames do not rapidly replay old screens.
    pub fn tick(&mut self, now_ms: u64) -> bool {
        if self.count > 1
            && self.pressed.is_none()
            && let Some(interval) = self.interval_ms
            && now_ms >= self.due_ms
        {
            self.index = (self.index + 1) % self.count;
            self.due_ms = now_ms.saturating_add(interval);
            self.pressed = None;
            true
        } else {
            false
        }
    }
}
