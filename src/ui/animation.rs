//! Animation engine for workspace button position transitions
//!
//! Target-chasing exponential interpolation: each button has a current and target
//! position. On each frame, current moves toward target with exponential ease-out.
//!
//! ## Spam Safety
//!
//! Each keypress just updates the target position. The animation is already running
//! and simply chases the new target. No queue, no blocking, no jank.
//! Rapid keypresses result in fluid retargeting — the button smoothly changes
//! direction mid-flight.
//!
//! ## Efficiency
//!
//! Tick callback only runs while any button is in motion. When all buttons settle
//! within the threshold (0.5px), the tick stops. Zero CPU when idle.

/// Per-button animation state
#[derive(Debug, Clone)]
pub struct ButtonAnimState {
    /// Current rendered position along primary axis (px)
    pub current: f64,
    /// Target position to animate toward (px)
    pub target: f64,
    /// Cached button width (px) — measured from preferred_width()
    pub width: f64,
}

impl ButtonAnimState {
    pub fn new(position: f64, width: f64) -> Self {
        Self {
            current: position,
            target: position,
            width,
        }
    }

    /// Whether this button has settled (within threshold of target)
    pub fn is_settled(&self, threshold: f64) -> bool {
        (self.current - self.target).abs() < threshold
    }
}

/// Animation engine for workspace button positions
///
/// Manages smooth transitions when buttons are reordered.
/// Uses exponential ease-out interpolation for snappy, responsive feel.
pub struct AnimationEngine {
    /// Per-button animation state, indexed by display position
    pub buttons: Vec<ButtonAnimState>,
    /// Interpolation speed (higher = faster settle)
    /// 12.0 → 63% in 83ms, 90% in 190ms, 99% in 380ms
    pub lerp_speed: f64,
    /// Settle threshold in pixels — below this, snap to target and stop
    pub settle_threshold: f64,
}

impl AnimationEngine {
    pub fn new() -> Self {
        Self {
            buttons: Vec::new(),
            lerp_speed: 12.0,
            settle_threshold: 0.5,
        }
    }

    /// Set target positions for all buttons.
    ///
    /// If `instant` is true, snaps current to target immediately (used on rebuild).
    /// If `instant` is false, current stays where it is and will animate toward target.
    ///
    /// `targets` is a slice of (target_position, width) tuples.
    pub fn set_targets(&mut self, targets: &[(f64, f64)], instant: bool) {
        // Resize if count changed
        while self.buttons.len() < targets.len() {
            self.buttons.push(ButtonAnimState::new(0.0, 0.0));
        }
        self.buttons.truncate(targets.len());

        for (i, &(target, width)) in targets.iter().enumerate() {
            self.buttons[i].target = target;
            self.buttons[i].width = width;
            if instant {
                self.buttons[i].current = target;
            }
        }
    }

    /// Simplified set_targets when widths haven't changed — just update positions.
    pub fn retarget(&mut self, targets: &[f64]) {
        for (i, &target) in targets.iter().enumerate() {
            if i < self.buttons.len() {
                self.buttons[i].target = target;
            }
        }
    }

    /// Advance one animation frame.
    ///
    /// `dt_secs` is the time since the last frame (typically ~0.016 for 60fps).
    /// Returns `true` if any button is still animating, `false` if all settled.
    pub fn tick(&mut self, dt_secs: f64) -> bool {
        let mut any_moving = false;

        for bs in &mut self.buttons {
            let delta = bs.target - bs.current;
            if delta.abs() < self.settle_threshold {
                // Snap to target
                bs.current = bs.target;
            } else {
                // Exponential ease-out: rapid initial movement, smooth deceleration
                // factor = 1 - e^(-speed * dt) ensures frame-rate independence
                let factor = 1.0 - (-self.lerp_speed * dt_secs).exp();
                bs.current += delta * factor;
                any_moving = true;
            }
        }

        any_moving
    }

    /// Whether any button is currently animating
    pub fn is_animating(&self) -> bool {
        self.buttons.iter().any(|b| !b.is_settled(self.settle_threshold))
    }

    /// Calculate target positions from button widths and spacing.
    /// Returns Vec of (target_x, width) tuples.
    pub fn compute_positions(widths: &[f64], spacing: f64) -> Vec<(f64, f64)> {
        let mut positions = Vec::with_capacity(widths.len());
        let mut x = 0.0;
        for &w in widths {
            positions.push((x, w));
            x += w + spacing;
        }
        positions
    }

    /// Total extent (width for horizontal, height for vertical) of all buttons
    pub fn total_extent(&self, spacing: f64) -> f64 {
        if self.buttons.is_empty() {
            return 0.0;
        }
        let last = self.buttons.last().unwrap();
        last.target + last.width + spacing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_instant_set() {
        let mut engine = AnimationEngine::new();
        engine.set_targets(&[(0.0, 40.0), (50.0, 40.0), (100.0, 40.0)], true);

        assert_eq!(engine.buttons.len(), 3);
        assert_eq!(engine.buttons[0].current, 0.0);
        assert_eq!(engine.buttons[1].current, 50.0);
        assert_eq!(engine.buttons[2].current, 100.0);
        assert!(!engine.is_animating());
    }

    #[test]
    fn test_animated_set() {
        let mut engine = AnimationEngine::new();
        // Start at positions 0, 50, 100
        engine.set_targets(&[(0.0, 40.0), (50.0, 40.0), (100.0, 40.0)], true);
        // Move button 0 to position 100, button 2 to position 0
        engine.retarget(&[100.0, 50.0, 0.0]);

        assert!(engine.is_animating());

        // Simulate 30 frames at 60fps (~500ms)
        for _ in 0..30 {
            engine.tick(1.0 / 60.0);
        }

        // Should be very close to targets
        assert!((engine.buttons[0].current - 100.0).abs() < 1.0);
        assert!((engine.buttons[2].current - 0.0).abs() < 1.0);
    }

    #[test]
    fn test_settles() {
        let mut engine = AnimationEngine::new();
        engine.set_targets(&[(50.0, 40.0)], true);
        engine.buttons[0].current = 0.0; // Start displaced

        // Run enough frames to settle
        let mut frames = 0;
        while engine.tick(1.0 / 60.0) {
            frames += 1;
            assert!(frames < 300, "animation didn't settle in 5 seconds");
        }

        assert_eq!(engine.buttons[0].current, 50.0);
        assert!(!engine.is_animating());
    }

    #[test]
    fn test_retarget_mid_animation() {
        let mut engine = AnimationEngine::new();
        engine.set_targets(&[(0.0, 40.0)], true);
        engine.buttons[0].current = 0.0;
        engine.buttons[0].target = 100.0;

        // Partial animation (5 frames)
        for _ in 0..5 {
            engine.tick(1.0 / 60.0);
        }
        let mid_pos = engine.buttons[0].current;
        assert!(mid_pos > 0.0 && mid_pos < 100.0, "should be mid-animation");

        // Retarget to opposite direction
        engine.retarget(&[-50.0]);

        // Should converge to new target
        for _ in 0..120 {
            engine.tick(1.0 / 60.0);
        }

        assert!((engine.buttons[0].current - (-50.0)).abs() < 0.5);
    }

    #[test]
    fn test_compute_positions() {
        let widths = vec![40.0, 40.0, 40.0];
        let positions = AnimationEngine::compute_positions(&widths, 4.0);
        assert_eq!(positions, vec![(0.0, 40.0), (44.0, 40.0), (88.0, 40.0)]);
    }

    #[test]
    fn test_compute_positions_varying_widths() {
        let widths = vec![30.0, 50.0, 20.0];
        let positions = AnimationEngine::compute_positions(&widths, 4.0);
        assert_eq!(positions, vec![(0.0, 30.0), (34.0, 50.0), (88.0, 20.0)]);
    }

    #[test]
    fn test_total_extent() {
        let mut engine = AnimationEngine::new();
        engine.set_targets(&[(0.0, 40.0), (44.0, 40.0), (88.0, 40.0)], true);
        // total = last.target (88) + last.width (40) + spacing (4) = 132
        assert_eq!(engine.total_extent(4.0), 132.0);
    }

    #[test]
    fn test_frame_rate_independence() {
        // Two engines: one at 60fps, one at 30fps, running for same real time
        let mut engine_60 = AnimationEngine::new();
        let mut engine_30 = AnimationEngine::new();

        engine_60.set_targets(&[(100.0, 40.0)], true);
        engine_60.buttons[0].current = 0.0;

        engine_30.set_targets(&[(100.0, 40.0)], true);
        engine_30.buttons[0].current = 0.0;

        // 60fps for 500ms = 30 frames
        for _ in 0..30 {
            engine_60.tick(1.0 / 60.0);
        }
        // 30fps for 500ms = 15 frames
        for _ in 0..15 {
            engine_30.tick(1.0 / 30.0);
        }

        // Should be approximately the same position (frame-rate independent)
        let diff = (engine_60.buttons[0].current - engine_30.buttons[0].current).abs();
        assert!(diff < 2.0, "60fps={} 30fps={} diff={}",
            engine_60.buttons[0].current, engine_30.buttons[0].current, diff);
    }

    #[test]
    fn test_empty_engine() {
        let mut engine = AnimationEngine::new();
        assert!(!engine.tick(1.0 / 60.0));
        assert!(!engine.is_animating());
        assert_eq!(engine.total_extent(4.0), 0.0);
    }
}
