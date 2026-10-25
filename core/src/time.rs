use std::fmt;

use thiserror::Error;

pub const PPQ: i64 = 983_040;
const MICROS_PER_SECOND: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Tick(i64);

impl Tick {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: i64) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frames(i64);

impl Frames {
    pub const ZERO: Self = Self(0);

    pub const fn from_raw(value: i64) -> Self {
        Self(value)
    }

    pub const fn raw(self) -> i64 {
        self.0
    }
}

impl fmt::Display for Frames {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TempoPoint {
    pub tick: Tick,
    pub micros_per_quarter: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSignature {
    pub tick: Tick,
    pub numerator: u16,
    pub denominator: u16,
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self {
            tick: Tick::ZERO,
            numerator: 4,
            denominator: 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub name: String,
    pub tick: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoopRegion {
    pub start: Tick,
    pub end: Tick,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickRange {
    pub start: Tick,
    pub end: Tick,
}

impl TickRange {
    pub fn len(self) -> Tick {
        Tick::from_raw(self.end.raw() - self.start.raw())
    }

    pub fn contains(self, tick: Tick) -> bool {
        self.start <= tick && tick < self.end
    }
}

#[derive(Clone, Copy, Debug)]
struct TempoSegment {
    point: TempoPoint,
    anchor_frames: Frames,
    remainder: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempoMap {
    sample_rate: u32,
    segments: crate::PersistentVector<TempoSegment>,
}

impl PartialEq for TempoSegment {
    fn eq(&self, other: &Self) -> bool {
        self.point == other.point
            && self.anchor_frames == other.anchor_frames
            && self.remainder == other.remainder
    }
}

impl Eq for TempoSegment {}

impl TempoMap {
    pub fn new(
        sample_rate: u32,
        points: impl IntoIterator<Item = TempoPoint>,
    ) -> Result<Self, TempoError> {
        if sample_rate == 0 {
            return Err(TempoError::ZeroSampleRate);
        }

        let points: Vec<TempoPoint> = points.into_iter().collect();
        let Some(first) = points.first().copied() else {
            return Err(TempoError::Empty);
        };
        if first.tick != Tick::ZERO {
            return Err(TempoError::FirstPointNotAtZero(first.tick));
        }

        let mut segments: crate::PersistentVector<TempoSegment> = crate::PersistentVector::new();
        for point in points {
            if point.micros_per_quarter == 0 {
                return Err(TempoError::ZeroTempo);
            }

            let (anchor_frames, remainder) = match segments.last() {
                None => (Frames::ZERO, 0),
                Some(previous) => {
                    if point.tick <= previous.point.tick {
                        return Err(TempoError::NotStrictlyIncreasing {
                            previous: previous.point.tick,
                            current: point.tick,
                        });
                    }
                    let numerator = tempo_numerator(
                        i128::from(point.tick.raw()) - i128::from(previous.point.tick.raw()),
                        previous.point.micros_per_quarter,
                        sample_rate,
                        previous.remainder,
                    )?;
                    let denominator = i128::from(PPQ) * i128::from(MICROS_PER_SECOND);
                    let frames = i64::try_from(numerator / denominator)
                        .map_err(|_| TempoError::OutOfRange)?;
                    let anchor = previous
                        .anchor_frames
                        .raw()
                        .checked_add(frames)
                        .ok_or(TempoError::OutOfRange)?;
                    (Frames::from_raw(anchor), numerator % denominator)
                }
            };

            segments.push_back(TempoSegment {
                point,
                anchor_frames,
                remainder,
            });
        }

        Ok(Self {
            sample_rate,
            segments,
        })
    }

    pub fn at_120_bpm(sample_rate: u32) -> Result<Self, TempoError> {
        Self::new(
            sample_rate,
            [TempoPoint {
                tick: Tick::ZERO,
                micros_per_quarter: 500_000,
            }],
        )
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn len(&self) -> usize {
        self.segments.len()
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn points(&self) -> impl Iterator<Item = TempoPoint> + '_ {
        self.segments.iter().map(|segment| segment.point)
    }

    pub fn frames_to_tick(&self, frames: Frames) -> Result<Tick, TempoError> {
        let segment = self
            .segments
            .iter()
            .rev()
            .find(|segment| segment.anchor_frames <= frames)
            .or_else(|| self.segments.front())
            .expect("a tempo map always has at least one segment");

        let delta_frames = i128::from(frames.raw()) - i128::from(segment.anchor_frames.raw());
        let numerator = delta_frames
            .checked_mul(i128::from(PPQ))
            .and_then(|value| value.checked_mul(i128::from(MICROS_PER_SECOND)))
            .ok_or(TempoError::OutOfRange)?;
        let denominator = i128::from(segment.point.micros_per_quarter)
            .checked_mul(i128::from(self.sample_rate))
            .ok_or(TempoError::OutOfRange)?;
        let ticks = i128::from(segment.point.tick.raw()) + numerator.div_euclid(denominator);
        i64::try_from(ticks)
            .map(Tick::from_raw)
            .map_err(|_| TempoError::OutOfRange)
    }

    pub fn tick_to_frames(&self, tick: Tick) -> Result<Frames, TempoError> {
        let segment = self
            .segments
            .iter()
            .rev()
            .find(|segment| segment.point.tick <= tick)
            .or_else(|| self.segments.front())
            .expect("a tempo map always has at least one segment");

        let numerator = tempo_numerator(
            i128::from(tick.raw()) - i128::from(segment.point.tick.raw()),
            segment.point.micros_per_quarter,
            self.sample_rate,
            segment.remainder,
        )?;
        let denominator = i128::from(PPQ) * i128::from(MICROS_PER_SECOND);
        let frames = i128::from(segment.anchor_frames.raw()) + numerator.div_euclid(denominator);
        i64::try_from(frames)
            .map(Frames::from_raw)
            .map_err(|_| TempoError::OutOfRange)
    }
}

fn tempo_numerator(
    delta_ticks: i128,
    micros_per_quarter: u32,
    sample_rate: u32,
    remainder: i128,
) -> Result<i128, TempoError> {
    delta_ticks
        .checked_mul(i128::from(micros_per_quarter))
        .and_then(|value| value.checked_mul(i128::from(sample_rate)))
        .and_then(|value| value.checked_add(remainder))
        .ok_or(TempoError::OutOfRange)
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TempoError {
    #[error("sample rate must be greater than zero")]
    ZeroSampleRate,
    #[error("tempo map must contain at least one point")]
    Empty,
    #[error("the first tempo point must be at tick 0, found tick {0}")]
    FirstPointNotAtZero(Tick),
    #[error("tempo points must be strictly increasing, found tick {previous} then {current}")]
    NotStrictlyIncreasing { previous: Tick, current: Tick },
    #[error("tempo must be greater than zero microseconds per quarter note")]
    ZeroTempo,
    #[error("musical position is outside the representable frame range")]
    OutOfRange,
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn constant(sample_rate: u32, micros_per_quarter: u32) -> TempoMap {
        TempoMap::new(
            sample_rate,
            [TempoPoint {
                tick: Tick::ZERO,
                micros_per_quarter,
            }],
        )
        .unwrap()
    }

    #[test]
    fn tick_ranges_are_half_open() {
        let range = TickRange {
            start: Tick::from_raw(10),
            end: Tick::from_raw(20),
        };
        assert!(range.contains(Tick::from_raw(10)));
        assert!(range.contains(Tick::from_raw(19)));
        assert!(!range.contains(Tick::from_raw(20)));
        assert_eq!(range.len(), Tick::from_raw(10));
    }

    #[test]
    fn tempo_at_120_bpm_gives_exact_beats() {
        let tempo = constant(48_000, 500_000);
        assert_eq!(tempo.tick_to_frames(Tick::ZERO), Ok(Frames::ZERO));
        assert_eq!(
            tempo.tick_to_frames(Tick::from_raw(PPQ)),
            Ok(Frames::from_raw(24_000))
        );
        assert_eq!(
            tempo.tick_to_frames(Tick::from_raw(4 * PPQ)),
            Ok(Frames::from_raw(96_000))
        );
    }

    #[test]
    fn tempo_changes_use_exact_segment_anchors() {
        let tempo = TempoMap::new(
            48_000,
            [
                TempoPoint {
                    tick: Tick::ZERO,
                    micros_per_quarter: 500_000,
                },
                TempoPoint {
                    tick: Tick::from_raw(4 * PPQ),
                    micros_per_quarter: 1_000_000,
                },
            ],
        )
        .unwrap();

        assert_eq!(
            tempo.tick_to_frames(Tick::from_raw(4 * PPQ)),
            Ok(Frames::from_raw(96_000))
        );
        assert_eq!(
            tempo.tick_to_frames(Tick::from_raw(8 * PPQ)),
            Ok(Frames::from_raw(288_000))
        );
    }

    #[test]
    fn frames_convert_back_to_ticks() {
        let tempo = constant(48_000, 500_000);
        assert_eq!(
            tempo.frames_to_tick(Frames::from_raw(24_000)),
            Ok(Tick::from_raw(PPQ))
        );
        assert_eq!(
            tempo.frames_to_tick(Frames::from_raw(96_000)),
            Ok(Tick::from_raw(4 * PPQ))
        );
    }

    #[test]
    fn conversion_is_seek_independent() {
        let tempo = constant(44_100, 487_000);
        let tick = Tick::from_raw(12_345);
        let first = tempo.tick_to_frames(tick).unwrap();
        let second = tempo.tick_to_frames(tick).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn negative_positions_extrapolate_from_the_first_segment() {
        let tempo = constant(48_000, 500_000);
        assert_eq!(
            tempo.tick_to_frames(Tick::from_raw(-PPQ)),
            Ok(Frames::from_raw(-24_000))
        );
    }

    #[test]
    fn rejects_invalid_tempo_maps() {
        let point = |tick, micros_per_quarter| TempoPoint {
            tick: Tick::from_raw(tick),
            micros_per_quarter,
        };

        assert_eq!(
            TempoMap::new(0, [point(0, 500_000)]),
            Err(TempoError::ZeroSampleRate)
        );
        assert_eq!(TempoMap::new(48_000, []), Err(TempoError::Empty));
        assert_eq!(
            TempoMap::new(48_000, [point(1, 500_000)]),
            Err(TempoError::FirstPointNotAtZero(Tick::from_raw(1)))
        );
        assert_eq!(
            TempoMap::new(48_000, [point(0, 0)]),
            Err(TempoError::ZeroTempo)
        );
        assert_eq!(
            TempoMap::new(48_000, [point(0, 500_000), point(0, 500_000)]),
            Err(TempoError::NotStrictlyIncreasing {
                previous: Tick::ZERO,
                current: Tick::ZERO,
            })
        );
    }

    proptest! {
        #[test]
        fn frames_are_monotonic(first in 0i64..10_000_000, second in 0i64..10_000_000) {
            let tempo = TempoMap::new(
                48_000,
                [
                    TempoPoint { tick: Tick::ZERO, micros_per_quarter: 500_000 },
                    TempoPoint { tick: Tick::from_raw(PPQ), micros_per_quarter: 333_333 },
                ],
            ).unwrap();
            let (low, high) = if first <= second { (first, second) } else { (second, first) };
            prop_assert!(tempo.tick_to_frames(Tick::from_raw(low)).unwrap()
                <= tempo.tick_to_frames(Tick::from_raw(high)).unwrap());
        }
    }
}
