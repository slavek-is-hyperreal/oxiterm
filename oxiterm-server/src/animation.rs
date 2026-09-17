//! Animation and Transition runtime controller for OxiTerm.
//!
//! Provides non-linear cubic Bézier solvers (Newton-Raphson), damped harmonic spring oscillators,
//! and 60 FPS property interpolation for animatable TCSS properties (`width`, `height`,
//! `top`, `left`, `right`, `bottom`, `opacity`).

use std::collections::HashMap;
use std::time::{Duration, Instant};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnimatableProperty, Easing};
use oxiterm_renderer::document::THTMLDocument;

/// Solves a cubic Bézier curve using Newton-Raphson iteration with bisection fallback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CubicBezierSolver {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl CubicBezierSolver {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Sample x coordinate at parameter t in [0, 1]
    #[inline]
    fn sample_x(&self, t: f32) -> f32 {
        // Polynomial: 3(1-t)^2 * t * x1 + 3(1-t) * t^2 * x2 + t^3
        let inv_t = 1.0 - t;
        3.0 * inv_t * inv_t * t * self.x1 + 3.0 * inv_t * t * t * self.x2 + t * t * t
    }

    /// Derivative of x coordinate with respect to t
    #[inline]
    fn sample_x_derivative(&self, t: f32) -> f32 {
        // d/dt: 3(1-t)^2 * x1 + 6(1-t)t * (x2 - x1) + 3t^2 * (1 - x2)
        let inv_t = 1.0 - t;
        3.0 * inv_t * inv_t * self.x1 + 6.0 * inv_t * t * (self.x2 - self.x1) + 3.0 * t * t * (1.0 - self.x2)
    }

    /// Sample y coordinate at parameter t in [0, 1]
    #[inline]
    fn sample_y(&self, t: f32) -> f32 {
        let inv_t = 1.0 - t;
        3.0 * inv_t * inv_t * t * self.y1 + 3.0 * inv_t * t * t * self.y2 + t * t * t
    }

    /// Solve for y given x in [0, 1]
    pub fn solve(&self, x: f32) -> f32 {
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }

        // Newton-Raphson
        let mut t = x;
        for _ in 0..8 {
            let current_x = self.sample_x(t) - x;
            if current_x.abs() < 1e-5 {
                return self.sample_y(t);
            }
            let d = self.sample_x_derivative(t);
            if d.abs() < 1e-6 {
                break;
            }
            t -= current_x / d;
            t = t.clamp(0.0, 1.0);
        }

        // Bisection fallback if Newton-Raphson did not fully converge
        let mut t0 = 0.0;
        let mut t1 = 1.0;
        t = x;

        while t0 < t1 {
            let current_x = self.sample_x(t);
            if (current_x - x).abs() < 1e-5 {
                return self.sample_y(t);
            }
            if x > current_x {
                t0 = t;
            } else {
                t1 = t;
            }
            t = (t1 + t0) * 0.5;
        }

        self.sample_y(t)
    }
}

/// Damped harmonic spring oscillator simulator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpringSolver {
    pub stiffness: f32,
    pub damping: f32,
    pub mass: f32,
}

impl SpringSolver {
    pub fn new(stiffness: f32, damping: f32, mass: f32) -> Self {
        let m = if mass <= 0.0 { 1.0 } else { mass };
        let k = if stiffness <= 0.0 { 100.0 } else { stiffness };
        let c = if damping < 0.0 { 10.0 } else { damping };
        Self { stiffness: k, damping: c, mass: m }
    }

    /// Returns (progress, is_settled) at time elapsed in seconds.
    pub fn solve(&self, t_secs: f32) -> (f32, bool) {
        if t_secs <= 0.0 {
            return (0.0, false);
        }

        let w0 = (self.stiffness / self.mass).sqrt();
        let zeta = self.damping / (2.0 * (self.mass * self.stiffness).sqrt());

        let (progress, velocity) = if zeta < 0.999 {
            // Underdamped
            let wd = w0 * (1.0 - zeta * zeta).sqrt();
            let decay = (-zeta * w0 * t_secs).exp();
            let cos_w = (wd * t_secs).cos();
            let sin_w = (wd * t_secs).sin();
            let envelope = cos_w + (zeta * w0 / wd) * sin_w;
            let val = 1.0 - decay * envelope;
            let vel = decay * (zeta * w0 * envelope + wd * sin_w - (zeta * w0 / wd) * wd * cos_w);
            (val, vel)
        } else if zeta <= 1.001 {
            // Critically damped
            let decay = (-w0 * t_secs).exp();
            let val = 1.0 - decay * (1.0 + w0 * t_secs);
            let vel = decay * w0 * w0 * t_secs;
            (val, vel)
        } else {
            // Overdamped
            let wd = w0 * (zeta * zeta - 1.0).sqrt();
            let decay = (-zeta * w0 * t_secs).exp();
            let cosh_w = (wd * t_secs).cosh();
            let sinh_w = (wd * t_secs).sinh();
            let envelope = cosh_w + (zeta * w0 / wd) * sinh_w;
            let val = 1.0 - decay * envelope;
            (val, 0.0)
        };

        // Settling criteria: close to 1.0 and slow velocity, or passed max spring duration (2.0s)
        let settled = ((progress - 1.0).abs() < 0.003 && velocity.abs() < 0.005) || t_secs > 2.0;
        if settled {
            (1.0, true)
        } else {
            (progress, false)
        }
    }
}

/// Evaluates an Easing function at normalized time progress [0, 1] or elapsed seconds.
pub fn evaluate_easing(easing: &Easing, t_ratio: f32, elapsed_secs: f32) -> (f32, bool) {
    match easing {
        Easing::Linear => (t_ratio.clamp(0.0, 1.0), t_ratio >= 1.0),
        Easing::Ease => {
            let solver = CubicBezierSolver::new(0.25, 0.1, 0.25, 1.0);
            (solver.solve(t_ratio), t_ratio >= 1.0)
        }
        Easing::EaseIn => {
            let solver = CubicBezierSolver::new(0.42, 0.0, 1.0, 1.0);
            (solver.solve(t_ratio), t_ratio >= 1.0)
        }
        Easing::EaseOut => {
            let solver = CubicBezierSolver::new(0.0, 0.0, 0.58, 1.0);
            (solver.solve(t_ratio), t_ratio >= 1.0)
        }
        Easing::EaseInOut => {
            let solver = CubicBezierSolver::new(0.42, 0.0, 0.58, 1.0);
            (solver.solve(t_ratio), t_ratio >= 1.0)
        }
        Easing::CubicBezier(x1, y1, x2, y2) => {
            let solver = CubicBezierSolver::new(*x1, *y1, *x2, *y2);
            (solver.solve(t_ratio), t_ratio >= 1.0)
        }
        Easing::Spring { stiffness, damping, mass } => {
            let solver = SpringSolver::new(*stiffness, *damping, *mass);
            solver.solve(elapsed_secs)
        }
    }
}

/// An active in-flight transition on a node's property.
#[derive(Debug, Clone)]
pub struct ActiveTransition {
    pub node_id: NodeId,
    pub property: AnimatableProperty,
    pub start_value: f32,
    pub end_value: f32,
    pub start_time: Instant,
    pub duration: Duration,
    pub delay: Duration,
    pub easing: Easing,
}

impl ActiveTransition {
    /// Samples the current interpolated value and whether the transition has finished.
    pub fn sample(&self, now: Instant) -> (f32, bool) {
        if now < self.start_time + self.delay {
            return (self.start_value, false);
        }

        let elapsed = now.duration_since(self.start_time + self.delay);
        let elapsed_secs = elapsed.as_secs_f32();
        let total_secs = self.duration.as_secs_f32();

        let t_ratio = if total_secs <= 0.0 {
            1.0
        } else {
            (elapsed_secs / total_secs).min(1.0)
        };

        let (progress, finished) = evaluate_easing(&self.easing, t_ratio, elapsed_secs);
        let current_val = self.start_value + (self.end_value - self.start_value) * progress;

        if finished {
            (self.end_value, true)
        } else {
            (current_val, false)
        }
    }
}

/// Cached baseline property values for change detection.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoredPropertyValues {
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub top: Option<i16>,
    pub left: Option<i16>,
    pub right: Option<i16>,
    pub bottom: Option<i16>,
    pub opacity: Option<f32>,
}

impl StoredPropertyValues {
    pub fn get_value(&self, prop: AnimatableProperty) -> Option<f32> {
        match prop {
            AnimatableProperty::Width => self.width.map(|v| v as f32),
            AnimatableProperty::Height => self.height.map(|v| v as f32),
            AnimatableProperty::Top => self.top.map(|v| v as f32),
            AnimatableProperty::Left => self.left.map(|v| v as f32),
            AnimatableProperty::Right => self.right.map(|v| v as f32),
            AnimatableProperty::Bottom => self.bottom.map(|v| v as f32),
            AnimatableProperty::Opacity => self.opacity,
            _ => None,
        }
    }

    pub fn set_value(&mut self, prop: AnimatableProperty, val: Option<f32>) {
        match prop {
            AnimatableProperty::Width => self.width = val.map(|v| v.round().max(0.0) as u16),
            AnimatableProperty::Height => self.height = val.map(|v| v.round().max(0.0) as u16),
            AnimatableProperty::Top => self.top = val.map(|v| v.round() as i16),
            AnimatableProperty::Left => self.left = val.map(|v| v.round() as i16),
            AnimatableProperty::Right => self.right = val.map(|v| v.round() as i16),
            AnimatableProperty::Bottom => self.bottom = val.map(|v| v.round() as i16),
            AnimatableProperty::Opacity => self.opacity = val.map(|v| v.clamp(0.0, 1.0)),
            _ => {}
        }
    }
}

/// Central animation and transition controller.
#[derive(Debug, Default)]
pub struct AnimationController {
    active_transitions: Vec<ActiveTransition>,
    baseline_styles: HashMap<NodeId, StoredPropertyValues>,
}

impl AnimationController {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether any transitions are currently actively animating.
    pub fn has_active(&self) -> bool {
        !self.active_transitions.is_empty()
    }

    /// Number of active transitions.
    pub fn active_count(&self) -> usize {
        self.active_transitions.len()
    }

    /// Clears all active transitions.
    pub fn clear(&mut self) {
        self.active_transitions.clear();
        self.baseline_styles.clear();
    }

    /// Cancels active transitions for a specific node.
    pub fn cancel_node(&mut self, node_id: NodeId) {
        self.active_transitions.retain(|t| t.node_id != node_id);
    }

    /// Cancels active transition for a specific property on a node.
    pub fn cancel_property(&mut self, node_id: NodeId, prop: AnimatableProperty) {
        self.active_transitions.retain(|t| !(t.node_id == node_id && t.property == prop));
    }

    /// Explicitly registers or overrides a transition on a node property.
    pub fn start_transition(
        &mut self,
        node_id: NodeId,
        property: AnimatableProperty,
        start_value: f32,
        end_value: f32,
        duration: Duration,
        delay: Duration,
        easing: Easing,
    ) {
        self.cancel_property(node_id, property);
        self.active_transitions.push(ActiveTransition {
            node_id,
            property,
            start_value,
            end_value,
            start_time: Instant::now(),
            duration,
            delay,
            easing,
        });
    }

    /// Detects style property changes on nodes with TCSS `transitions` specs,
    /// launching in-flight transitions from current value to new target value.
    pub fn sync_transitions(&mut self, doc: &mut THTMLDocument, now: Instant) {
        let mut new_transitions = Vec::new();

        for (node_id, node) in doc.arena.iter() {
            if node.style.transitions.is_empty() {
                continue;
            }

            let mut stored = self.baseline_styles.entry(node_id).or_default().clone();

            for spec in &node.style.transitions {
                let props_to_check: Vec<AnimatableProperty> = match spec.property {
                    AnimatableProperty::All => vec![
                        AnimatableProperty::Width,
                        AnimatableProperty::Height,
                        AnimatableProperty::Top,
                        AnimatableProperty::Left,
                        AnimatableProperty::Right,
                        AnimatableProperty::Bottom,
                        AnimatableProperty::Opacity,
                    ],
                    other => vec![other],
                };

                for prop in props_to_check {
                    let target_val = match prop {
                        AnimatableProperty::Width => node.style.width.map(|v| v as f32),
                        AnimatableProperty::Height => node.style.height.map(|v| v as f32),
                        AnimatableProperty::Top => node.style.top.map(|v| v as f32),
                        AnimatableProperty::Left => node.style.left.map(|v| v as f32),
                        AnimatableProperty::Right => node.style.right.map(|v| v as f32),
                        AnimatableProperty::Bottom => node.style.bottom.map(|v| v as f32),
                        AnimatableProperty::Opacity => node.style.opacity,
                        _ => None,
                    };

                    if let Some(target) = target_val {
                        let prev = stored.get_value(prop);
                        match prev {
                            None => {
                                // First time recorded: baseline established, no transition
                                stored.set_value(prop, Some(target));
                            }
                            Some(prev_val) => {
                                if (prev_val - target).abs() > 0.001 {
                                    // Target changed! Check if already transitioning
                                    let current_start = self.get_current_in_flight_value(node_id, prop, now)
                                        .unwrap_or(prev_val);

                                    new_transitions.push((
                                        node_id,
                                        prop,
                                        current_start,
                                        target,
                                        Duration::from_millis(spec.duration_ms as u64),
                                        Duration::from_millis(spec.delay_ms as u64),
                                        spec.easing,
                                    ));
                                    stored.set_value(prop, Some(target));
                                }
                            }
                        }
                    }
                }
            }

            self.baseline_styles.insert(node_id, stored);
        }

        for (node_id, prop, start, end, dur, delay, easing) in new_transitions {
            self.start_transition(node_id, prop, start, end, dur, delay, easing);
        }
    }

    /// Restores node styles to their target baselines before processing state changes or computing new transitions.
    ///
    /// This prevents intermediate in-flight animation values from `tick()` being mistaken for external style changes.
    pub fn restore_baselines(&self, doc: &mut THTMLDocument) {
        for (node_id, stored) in &self.baseline_styles {
            if let Some(node) = doc.arena.get_mut(*node_id) {
                if let Some(w) = stored.width {
                    node.style.width = Some(w);
                }
                if let Some(h) = stored.height {
                    node.style.height = Some(h);
                }
                if let Some(t) = stored.top {
                    node.style.top = Some(t);
                }
                if let Some(l) = stored.left {
                    node.style.left = Some(l);
                }
                if let Some(r) = stored.right {
                    node.style.right = Some(r);
                }
                if let Some(b) = stored.bottom {
                    node.style.bottom = Some(b);
                }
                if let Some(o) = stored.opacity {
                    node.style.opacity = Some(o);
                }
            }
        }
    }

    /// Gets current in-flight value if the property is already transitioning.
    fn get_current_in_flight_value(&self, node_id: NodeId, prop: AnimatableProperty, now: Instant) -> Option<f32> {
        self.active_transitions
            .iter()
            .find(|t| t.node_id == node_id && t.property == prop)
            .map(|t| t.sample(now).0)
    }

    /// Advances all active transitions to `now`, writing interpolated values into `doc`.
    ///
    /// Returns `true` if any document node styles were modified.
    pub fn tick(&mut self, doc: &mut THTMLDocument, now: Instant) -> bool {
        if self.active_transitions.is_empty() {
            return false;
        }

        let mut changed = false;
        let mut i = 0;

        while i < self.active_transitions.len() {
            let (val, finished) = self.active_transitions[i].sample(now);
            let node_id = self.active_transitions[i].node_id;
            let prop = self.active_transitions[i].property;

            if let Some(node) = doc.arena.get_mut(node_id) {
                let style_modified = match prop {
                    AnimatableProperty::Width => {
                        let new_val = Some(val.round().max(0.0) as u16);
                        if node.style.width != new_val {
                            node.style.width = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Height => {
                        let new_val = Some(val.round().max(0.0) as u16);
                        if node.style.height != new_val {
                            node.style.height = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Top => {
                        let new_val = Some(val.round() as i16);
                        if node.style.top != new_val {
                            node.style.top = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Left => {
                        let new_val = Some(val.round() as i16);
                        if node.style.left != new_val {
                            node.style.left = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Right => {
                        let new_val = Some(val.round() as i16);
                        if node.style.right != new_val {
                            node.style.right = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Bottom => {
                        let new_val = Some(val.round() as i16);
                        if node.style.bottom != new_val {
                            node.style.bottom = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    AnimatableProperty::Opacity => {
                        let new_val = Some(val.clamp(0.0, 1.0));
                        if node.style.opacity != new_val {
                            node.style.opacity = new_val;
                            true
                        } else {
                            false
                        }
                    }
                    _ => false,
                };

                if style_modified {
                    doc.mark_dirty(node_id);
                    changed = true;
                }
            }

            if finished {
                self.active_transitions.swap_remove(i);
            } else {
                i += 1;
            }
        }

        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_proto::style::TransitionSpec;

    #[test]
    fn test_cubic_bezier_endpoints() {
        let solver = CubicBezierSolver::new(0.25, 0.1, 0.25, 1.0);
        assert_eq!(solver.solve(0.0), 0.0);
        assert_eq!(solver.solve(1.0), 1.0);
    }

    #[test]
    fn test_cubic_bezier_ease_out() {
        let solver = CubicBezierSolver::new(0.0, 0.0, 0.58, 1.0);
        let mid = solver.solve(0.5);
        // Ease-out should have higher progress at midpoint than linear 0.5
        assert!(mid > 0.5, "midpoint should be > 0.5 for ease-out, got {}", mid);
    }

    #[test]
    fn test_cubic_bezier_ease_in() {
        let solver = CubicBezierSolver::new(0.42, 0.0, 1.0, 1.0);
        let mid = solver.solve(0.5);
        // Ease-in should have lower progress at midpoint than linear 0.5
        assert!(mid < 0.5, "midpoint should be < 0.5 for ease-in, got {}", mid);
    }

    #[test]
    fn test_spring_oscillator_settles() {
        let spring = SpringSolver::new(100.0, 10.0, 1.0);
        let (val_start, _) = spring.solve(0.0);
        assert_eq!(val_start, 0.0);

        let (val_later, settled) = spring.solve(1.5);
        assert!(settled, "Spring should settle after 1.5s");
        assert_eq!(val_later, 1.0);
    }

    #[test]
    fn test_animation_controller_tick() {
        let mut doc = THTMLDocument::default();
        let mut node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Box);
        node.style.width = Some(10);
        node.style.transitions.push(TransitionSpec {
            property: AnimatableProperty::Width,
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
        });
        let id = doc.arena.alloc(node);
        doc.root = id;

        let mut controller = AnimationController::new();
        let now = Instant::now();

        // Baseline establishment
        controller.sync_transitions(&mut doc, now);
        assert!(!controller.has_active());

        // Change width to 20
        doc.arena.get_mut(id).unwrap().style.width = Some(20);
        controller.sync_transitions(&mut doc, now);
        assert!(controller.has_active());
        assert_eq!(controller.active_count(), 1);

        // Tick at 50ms (halfway)
        let mid_time = now + Duration::from_millis(50);
        let changed = controller.tick(&mut doc, mid_time);
        assert!(changed);
        let current_width = doc.arena.get(id).unwrap().style.width.unwrap();
        assert!(current_width >= 14 && current_width <= 16, "Width should be ~15, got {}", current_width);

        // Tick at 110ms (finished)
        let end_time = now + Duration::from_millis(110);
        let changed_end = controller.tick(&mut doc, end_time);
        assert!(changed_end);
        let final_width = doc.arena.get(id).unwrap().style.width.unwrap();
        assert_eq!(final_width, 20);
        assert!(!controller.has_active());
    }

    #[test]
    fn test_restore_baselines_prevents_intermediate_target_corruption() {
        let mut doc = THTMLDocument::default();
        let mut node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Box);
        node.style.width = Some(10);
        node.style.transitions.push(TransitionSpec {
            property: AnimatableProperty::Width,
            duration_ms: 100,
            delay_ms: 0,
            easing: Easing::Linear,
        });
        let id = doc.arena.alloc(node);
        doc.root = id;

        let mut controller = AnimationController::new();
        let now = Instant::now();

        // Baseline: width 10
        controller.sync_transitions(&mut doc, now);

        // New target: width 20
        doc.arena.get_mut(id).unwrap().style.width = Some(20);
        controller.sync_transitions(&mut doc, now);
        assert!(controller.has_active());

        // Halfway tick: sets node.style.width to 15
        controller.tick(&mut doc, now + Duration::from_millis(50));
        assert_eq!(doc.arena.get(id).unwrap().style.width, Some(15));

        // Restore baselines: node.style.width restored to target 20
        controller.restore_baselines(&mut doc);
        assert_eq!(doc.arena.get(id).unwrap().style.width, Some(20));

        // Calling sync_transitions again does NOT spawn extra transitions or alter target 20
        controller.sync_transitions(&mut doc, now + Duration::from_millis(50));
        assert_eq!(controller.active_count(), 1);
    }
}
