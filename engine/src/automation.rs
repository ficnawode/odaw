use odaw_core::{AutomationMode, Curve, ParamAddress, ParamValue};

use crate::param::ParamBuffer;

#[derive(Clone, Debug, Default)]
pub struct AutomationPlan {
    pub lanes: Vec<LanePlan>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LanePlan {
    pub address: ParamAddress,
    pub slot: usize,
    pub mode: AutomationMode,
    pub points: Vec<PointPlan>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointPlan {
    pub frame: u64,
    pub value: ParamValue,
    pub curve: Curve,
}

impl AutomationPlan {
    pub fn is_empty(&self) -> bool {
        self.lanes.is_empty()
    }

    /// The next automation point after `frame`, or `frame + limit` when the
    /// chunk has no point inside it.
    pub fn next_boundary(&self, frame: u64, limit: usize) -> usize {
        let end = frame.saturating_add(limit as u64);
        let mut boundary = end;
        for lane in &self.lanes {
            let index = lane.points.partition_point(|point| point.frame <= frame);
            if let Some(point) = lane.points.get(index) {
                boundary = boundary.min(point.frame);
            }
        }
        boundary.saturating_sub(frame).min(limit as u64) as usize
    }

    /// Apply every active lane to the effective parameter values for this chunk.
    pub fn apply(&self, frame: u64, frames: usize, params: &mut ParamBuffer) {
        for lane in &self.lanes {
            if params.is_touched(lane.slot) {
                continue;
            }
            let Some(start) = lane.value_at(frame) else {
                continue;
            };
            let end = lane
                .value_at(frame.saturating_add(frames as u64))
                .unwrap_or(start);
            let base = params.base(&lane.address).unwrap_or(start);
            let start = combine(lane.mode, base, start);
            let end = combine(lane.mode, base, end);
            if lane.curve_at(frame) == Curve::Linear
                && frames > 1
                && start != end
                && matches!(start, ParamValue::Float(_))
            {
                params.set_ramp(&lane.address, start.as_f64(), end.as_f64());
            } else {
                params.set_effective(&lane.address, start);
            }
        }
    }
}

impl LanePlan {
    pub fn value_at(&self, frame: u64) -> Option<ParamValue> {
        let first = self.points.first()?;
        if frame <= first.frame {
            return Some(first.value);
        }
        let last = self.points.last()?;
        if frame >= last.frame {
            return Some(last.value);
        }
        let index = self.points.partition_point(|point| point.frame <= frame);
        let previous = self.points.get(index.saturating_sub(1))?;
        let next = self.points.get(index)?;
        if previous.curve == Curve::Hold
            || !previous.value.is_continuous()
            || !next.value.is_continuous()
        {
            return Some(previous.value);
        }
        let span = next.frame.saturating_sub(previous.frame).max(1);
        let ratio = frame.saturating_sub(previous.frame) as f64 / span as f64;
        Some(previous.value.interpolate(next.value, ratio))
    }

    pub fn curve_at(&self, frame: u64) -> Curve {
        let Some(first) = self.points.first() else {
            return Curve::Hold;
        };
        if frame < first.frame {
            return Curve::Hold;
        }
        let index = self.points.partition_point(|point| point.frame <= frame);
        self.points
            .get(index.saturating_sub(1))
            .map_or(Curve::Hold, |point| point.curve)
    }
}

fn combine(mode: AutomationMode, base: ParamValue, value: ParamValue) -> ParamValue {
    match mode {
        AutomationMode::Absolute => value,
        AutomationMode::Trim => base.offset(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use odaw_core::{NodeId, ParamId};

    fn address() -> ParamAddress {
        ParamAddress {
            node: NodeId::from_raw(1),
            param: ParamId::new("gain"),
        }
    }

    fn lane(mode: AutomationMode, points: Vec<PointPlan>) -> LanePlan {
        LanePlan {
            address: address(),
            slot: 0,
            mode,
            points,
        }
    }

    #[test]
    fn values_hold_before_and_after_and_interpolate_linearly() {
        let lane = lane(
            AutomationMode::Absolute,
            vec![
                PointPlan {
                    frame: 100,
                    value: ParamValue::Float(0.0),
                    curve: Curve::Linear,
                },
                PointPlan {
                    frame: 200,
                    value: ParamValue::Float(1.0),
                    curve: Curve::Linear,
                },
            ],
        );
        assert_eq!(lane.value_at(0), Some(ParamValue::Float(0.0)));
        assert_eq!(lane.value_at(150), Some(ParamValue::Float(0.5)));
        assert_eq!(lane.value_at(250), Some(ParamValue::Float(1.0)));
    }

    #[test]
    fn hold_ignores_interpolation() {
        let lane = lane(
            AutomationMode::Absolute,
            vec![
                PointPlan {
                    frame: 0,
                    value: ParamValue::Float(0.0),
                    curve: Curve::Hold,
                },
                PointPlan {
                    frame: 100,
                    value: ParamValue::Float(1.0),
                    curve: Curve::Linear,
                },
            ],
        );
        assert_eq!(lane.value_at(99), Some(ParamValue::Float(0.0)));
        assert_eq!(lane.value_at(100), Some(ParamValue::Float(1.0)));
    }

    #[test]
    fn apply_writes_a_ramp_for_linear_float_lanes() {
        let plan = AutomationPlan {
            lanes: vec![lane(
                AutomationMode::Absolute,
                vec![
                    PointPlan {
                        frame: 0,
                        value: ParamValue::Float(0.0),
                        curve: Curve::Linear,
                    },
                    PointPlan {
                        frame: 100,
                        value: ParamValue::Float(1.0),
                        curve: Curve::Linear,
                    },
                ],
            )],
        };
        let mut params = ParamBuffer::new();
        params.ensure(address(), ParamValue::Float(0.0));
        plan.apply(0, 50, &mut params);
        assert_eq!(params.ramp(&address()), Some((0.0, 0.5)));
    }

    #[test]
    fn trim_adds_the_lane_to_the_base() {
        let plan = AutomationPlan {
            lanes: vec![lane(
                AutomationMode::Trim,
                vec![PointPlan {
                    frame: 0,
                    value: ParamValue::Float(3.0),
                    curve: Curve::Hold,
                }],
            )],
        };
        let mut params = ParamBuffer::new();
        params.ensure(address(), ParamValue::Float(-6.0));
        plan.apply(0, 64, &mut params);
        assert_eq!(params.get(&address()), Some(ParamValue::Float(-3.0)));
        assert_eq!(params.base(&address()), Some(ParamValue::Float(-6.0)));
    }

    #[test]
    fn boundaries_are_the_next_point_or_the_limit() {
        let plan = AutomationPlan {
            lanes: vec![lane(
                AutomationMode::Absolute,
                vec![
                    PointPlan {
                        frame: 100,
                        value: ParamValue::Float(0.0),
                        curve: Curve::Linear,
                    },
                    PointPlan {
                        frame: 300,
                        value: ParamValue::Float(1.0),
                        curve: Curve::Linear,
                    },
                ],
            )],
        };
        assert_eq!(plan.next_boundary(0, 256), 100);
        assert_eq!(plan.next_boundary(100, 256), 200);
        assert_eq!(plan.next_boundary(300, 256), 256);
        assert_eq!(plan.next_boundary(50, 100), 50);
    }
}

#[cfg(test)]
mod ramp_tests {
    use super::*;
    use odaw_core::{NodeId, ParamId};

    fn address() -> ParamAddress {
        ParamAddress {
            node: NodeId::from_raw(1),
            param: ParamId::new("gain"),
        }
    }

    #[test]
    fn ramp_values_are_aligned_across_blocks() {
        let plan = AutomationPlan {
            lanes: vec![LanePlan {
                address: address(),
                slot: 0,
                mode: AutomationMode::Absolute,
                points: vec![
                    PointPlan {
                        frame: 0,
                        value: ParamValue::Float(0.0),
                        curve: Curve::Linear,
                    },
                    PointPlan {
                        frame: 24_000,
                        value: ParamValue::Float(-60.0),
                        curve: Curve::Linear,
                    },
                ],
            }],
        };
        let mut wide = ParamBuffer::new();
        wide.ensure(address(), ParamValue::Float(0.0));
        plan.apply(0, 64, &mut wide);
        let wide_value = wide.float_at(&address(), 32, 64).unwrap();

        let mut narrow = ParamBuffer::new();
        narrow.ensure(address(), ParamValue::Float(0.0));
        plan.apply(32, 32, &mut narrow);
        let narrow_value = narrow.float_at(&address(), 0, 32).unwrap();
        assert!((wide_value - narrow_value).abs() < 1e-9);
    }
}
