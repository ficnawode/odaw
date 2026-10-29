use thiserror::Error;

use crate::graph::{AutomationLane, AutomationMode, Curve, Keyframe};
use crate::params::ParamValue;
use crate::time::Tick;

impl AutomationLane {
    pub fn is_active(&self) -> bool {
        self.enabled && !self.keyframes.is_empty()
    }

    /// The curve value at `tick`. The caller decides whether a disabled lane
    /// participates; this is the pure curve, useful for drawing and editing.
    pub fn value_at(&self, tick: Tick) -> Option<ParamValue> {
        let first = self.keyframes.front()?;
        if tick <= first.tick {
            return Some(first.value);
        }
        let last = self.keyframes.back()?;
        if tick >= last.tick {
            return Some(last.value);
        }

        let mut previous = first;
        for point in &self.keyframes {
            if point.tick > tick {
                break;
            }
            previous = point;
        }
        let Some(next) = self
            .keyframes
            .iter()
            .find(|point| point.tick > previous.tick)
        else {
            return Some(previous.value);
        };
        if previous.curve == Curve::Hold
            || !previous.value.is_continuous()
            || !next.value.is_continuous()
        {
            return Some(previous.value);
        }
        let span = (next.tick.raw() - previous.tick.raw()).max(1);
        let ratio = (tick.raw() - previous.tick.raw()) as f64 / span as f64;
        Some(previous.value.interpolate(next.value, ratio))
    }

    /// Sort, clamp, and de-duplicate points. Equal ticks keep the last point
    /// in input order. Fails on non-finite floats and mixed value variants.
    pub fn normalized(&self) -> Result<Self, AutomationError> {
        let mut points = Vec::with_capacity(self.keyframes.len());
        let mut kind = None;
        for point in &self.keyframes {
            if let ParamValue::Float(value) = point.value {
                if !value.is_finite() {
                    return Err(AutomationError::NonFiniteValue);
                }
            }
            match kind {
                Some(existing) if existing != value_kind(point.value) => {
                    return Err(AutomationError::MixedValueKinds);
                }
                None => kind = Some(value_kind(point.value)),
                _ => {}
            }
            points.push(Keyframe {
                tick: Tick::from_raw(point.tick.raw().max(0)),
                ..*point
            });
        }
        points.sort_by_key(|point| point.tick);

        let mut deduped: Vec<Keyframe> = Vec::with_capacity(points.len());
        for point in points {
            if deduped
                .last()
                .is_some_and(|previous| previous.tick == point.tick)
            {
                deduped.pop();
            }
            deduped.push(point);
        }

        let mut lane = self.clone();
        lane.keyframes = deduped.into_iter().collect();
        Ok(lane)
    }

    /// Reject a stored lane that is not normalized or uses an impossible mode.
    pub fn validate(&self) -> Result<(), AutomationError> {
        let mut previous: Option<Tick> = None;
        let mut kind = None;
        for point in &self.keyframes {
            if point.tick < Tick::ZERO {
                return Err(AutomationError::NegativeTick);
            }
            if previous.is_some_and(|tick| point.tick <= tick) {
                return Err(AutomationError::NotStrictlyIncreasing);
            }
            previous = Some(point.tick);
            if let ParamValue::Float(value) = point.value {
                if !value.is_finite() {
                    return Err(AutomationError::NonFiniteValue);
                }
            }
            match kind {
                Some(existing) if existing != value_kind(point.value) => {
                    return Err(AutomationError::MixedValueKinds);
                }
                None => kind = Some(value_kind(point.value)),
                _ => {}
            }
        }
        if self.mode == AutomationMode::Trim
            && kind.is_some_and(|kind| kind != ValueKind::Float && kind != ValueKind::Int)
        {
            return Err(AutomationError::TrimOnDiscrete);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValueKind {
    Float,
    Int,
    Bool,
    Enum,
}

fn value_kind(value: ParamValue) -> ValueKind {
    match value {
        ParamValue::Float(_) => ValueKind::Float,
        ParamValue::Int(_) => ValueKind::Int,
        ParamValue::Bool(_) => ValueKind::Bool,
        ParamValue::Enum(_) => ValueKind::Enum,
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum AutomationError {
    #[error("automation values must be finite")]
    NonFiniteValue,
    #[error("automation points must use one value kind")]
    MixedValueKinds,
    #[error("automation tick is negative")]
    NegativeTick,
    #[error("automation points must be strictly increasing by tick")]
    NotStrictlyIncreasing,
    #[error("trim automation is only valid for float and integer parameters")]
    TrimOnDiscrete,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lane(points: impl IntoIterator<Item = (i64, ParamValue)>) -> AutomationLane {
        AutomationLane {
            mode: AutomationMode::Absolute,
            enabled: true,
            keyframes: points
                .into_iter()
                .map(|(tick, value)| Keyframe {
                    tick: Tick::from_raw(tick),
                    value,
                    curve: Curve::Linear,
                })
                .collect(),
        }
    }

    #[test]
    fn values_hold_before_the_first_and_after_the_last_point() {
        let lane = make_lane([
            (100, ParamValue::Float(0.25)),
            (200, ParamValue::Float(0.75)),
        ]);
        assert_eq!(lane.value_at(Tick::ZERO), Some(ParamValue::Float(0.25)));
        assert_eq!(
            lane.value_at(Tick::from_raw(100)),
            Some(ParamValue::Float(0.25))
        );
        assert_eq!(
            lane.value_at(Tick::from_raw(150)),
            Some(ParamValue::Float(0.5))
        );
        assert_eq!(
            lane.value_at(Tick::from_raw(200)),
            Some(ParamValue::Float(0.75))
        );
        assert_eq!(
            lane.value_at(Tick::from_raw(900)),
            Some(ParamValue::Float(0.75))
        );
    }

    #[test]
    fn hold_steps_at_the_next_point() {
        let mut lane = make_lane([(0, ParamValue::Float(0.0)), (100, ParamValue::Float(1.0))]);
        lane.keyframes.get_mut(0).unwrap().curve = Curve::Hold;
        assert_eq!(
            lane.value_at(Tick::from_raw(99)),
            Some(ParamValue::Float(0.0))
        );
        assert_eq!(
            lane.value_at(Tick::from_raw(100)),
            Some(ParamValue::Float(1.0))
        );
    }

    #[test]
    fn integers_round_and_discrete_values_hold() {
        let lane = make_lane([(0, ParamValue::Int(0)), (10, ParamValue::Int(10))]);
        assert_eq!(lane.value_at(Tick::from_raw(4)), Some(ParamValue::Int(4)));
        assert_eq!(lane.value_at(Tick::from_raw(6)), Some(ParamValue::Int(6)));

        let lane = make_lane([(0, ParamValue::Bool(false)), (10, ParamValue::Bool(true))]);
        assert_eq!(
            lane.value_at(Tick::from_raw(9)),
            Some(ParamValue::Bool(false))
        );
    }

    #[test]
    fn normalization_sorts_clamps_and_keeps_the_last_duplicate() {
        let lane = make_lane([
            (100, ParamValue::Float(1.0)),
            (-5, ParamValue::Float(-1.0)),
            (100, ParamValue::Float(2.0)),
        ]);
        let normalized = lane.normalized().unwrap();
        assert_eq!(normalized.keyframes.len(), 2);
        assert_eq!(normalized.keyframes[0].tick, Tick::ZERO);
        assert_eq!(normalized.keyframes[1].value, ParamValue::Float(2.0));
    }

    #[test]
    fn validation_rejects_unsorted_and_mixed_lanes() {
        let lane = make_lane([(10, ParamValue::Float(1.0)), (5, ParamValue::Float(2.0))]);
        assert_eq!(lane.validate(), Err(AutomationError::NotStrictlyIncreasing));

        let mut lane = make_lane([(0, ParamValue::Float(1.0))]);
        lane.keyframes.push_back(Keyframe {
            tick: Tick::from_raw(1),
            value: ParamValue::Int(1),
            curve: Curve::Linear,
        });
        assert_eq!(lane.validate(), Err(AutomationError::MixedValueKinds));
    }

    #[test]
    fn trim_is_rejected_for_discrete_lanes() {
        let mut lane = make_lane([(0, ParamValue::Bool(true))]);
        lane.mode = AutomationMode::Trim;
        assert_eq!(lane.validate(), Err(AutomationError::TrimOnDiscrete));
    }

    #[test]
    fn disabled_lanes_are_not_active() {
        let mut lane = make_lane([(0, ParamValue::Float(1.0))]);
        lane.enabled = false;
        assert!(!lane.is_active());
        assert_eq!(lane.value_at(Tick::ZERO), Some(ParamValue::Float(1.0)));
    }
}
