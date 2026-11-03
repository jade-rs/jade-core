//! Exponential triple smoothing as FORECAST.ETS and its family compute it:
//! the AAA model (additive error, trend and seasonality).
//!
//! Excel does not publish how it starts the smoothing or searches for its
//! parameters, so this follows the classical method: the first season's mean
//! and the change to the second for level and trend, the mean deviation at
//! each position of the season for the seasonal indices, and α, β and γ
//! chosen between 0.001 and 0.999 to minimise the squared one-step errors.
//! Against Excel's results for trending and seasonal series the forecasts
//! agree to within about 3% (mostly well under 1%); α, β and γ themselves
//! may differ, since many of them fit almost equally well.
//!
//! What Excel documents is followed as it says: the timeline sorted, points
//! on the same date aggregated, a missing point completed from its
//! neighbours or taken as zero, a constant step required (whole months for
//! monthly dates), and seasonality detected unless given.

use crate::formatter::dates::from_excel_date;
use chrono::Datelike;

/// Why a series cannot be smoothed, as the error the functions return.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EtsError {
    /// `#N/A`: the values and the timeline differ in size.
    Size,
    /// `#NUM!`: no constant step, too few points, or an argument out of range.
    Number,
}

/// How points that share a date are combined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Aggregation {
    Average,
    Count,
    CountA,
    Max,
    Median,
    Min,
    Sum,
}

impl Aggregation {
    pub(crate) fn from_code(code: f64) -> Option<Aggregation> {
        Some(match code.trunc() as i64 {
            1 => Aggregation::Average,
            2 => Aggregation::Count,
            3 => Aggregation::CountA,
            4 => Aggregation::Max,
            5 => Aggregation::Median,
            6 => Aggregation::Min,
            7 => Aggregation::Sum,
            _ => return None,
        })
    }

    fn apply(self, values: &mut [f64]) -> Option<f64> {
        if values.is_empty() {
            return match self {
                Aggregation::Count | Aggregation::CountA => Some(0.0),
                _ => None,
            };
        }
        let n = values.len() as f64;
        Some(match self {
            Aggregation::Average => values.iter().sum::<f64>() / n,
            Aggregation::Count | Aggregation::CountA => n,
            Aggregation::Max => values.iter().cloned().fold(f64::MIN, f64::max),
            Aggregation::Min => values.iter().cloned().fold(f64::MAX, f64::min),
            Aggregation::Sum => values.iter().sum(),
            Aggregation::Median => {
                values.sort_by(|a, b| a.total_cmp(b));
                let middle = values.len() / 2;
                if values.len().is_multiple_of(2) {
                    (values[middle - 1] + values[middle]) / 2.0
                } else {
                    values[middle]
                }
            }
        })
    }
}

/// How positions on the regular series map back to the timeline.
#[derive(Debug, Clone, PartialEq)]
enum Steps {
    /// A constant step in the timeline's own units from `start`.
    Even { start: f64, step: f64 },
    /// Dates a whole number of months apart, on the same day of the month.
    Months {
        start_month: i64,
        start_day: u32,
        months: i64,
    },
}

/// A series on a regular timeline, gaps completed.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Series {
    pub(crate) values: Vec<f64>,
    steps: Steps,
    /// The step size FORECAST.ETS.STAT reports.
    pub(crate) step_size: f64,
}

fn month_of(serial: f64) -> Option<(i64, u32, u32)> {
    if serial.fract() != 0.0 {
        return None;
    }
    let date = from_excel_date(serial as i64).ok()?;
    let days_in_month = days_in_month(date.year(), date.month());
    Some((
        date.year() as i64 * 12 + date.month0() as i64,
        date.day(),
        days_in_month,
    ))
}

fn days_in_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1);
    let next = chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1);
    match (first, next) {
        (Some(first), Some(next)) => (next - first).num_days() as u32,
        _ => 30,
    }
}

fn gcd(a: i64, b: i64) -> i64 {
    if b == 0 {
        a.abs()
    } else {
        gcd(b, a % b)
    }
}

impl Series {
    /// Sorts, aggregates and completes `values` along `timeline`.
    pub(crate) fn new(
        values: &[Option<f64>],
        timeline: &[Option<f64>],
        complete: bool,
        aggregation: Aggregation,
    ) -> Result<Series, EtsError> {
        if values.len() != timeline.len() {
            return Err(EtsError::Size);
        }
        let mut points: Vec<(f64, Option<f64>)> = timeline
            .iter()
            .zip(values)
            .filter_map(|(time, value)| time.map(|time| (time, *value)))
            .collect();
        points.sort_by(|a, b| a.0.total_cmp(&b.0));
        // One value per date.
        let mut dates: Vec<f64> = Vec::new();
        let mut aggregated: Vec<Option<f64>> = Vec::new();
        let mut index = 0;
        while index < points.len() {
            let date = points[index].0;
            let mut group = Vec::new();
            while index < points.len() && points[index].0 == date {
                if let Some(value) = points[index].1 {
                    group.push(value);
                }
                index += 1;
            }
            dates.push(date);
            aggregated.push(aggregation.apply(&mut group));
        }
        if dates.len() < 2 {
            return Err(EtsError::Number);
        }
        let differences: Vec<f64> = dates.windows(2).map(|pair| pair[1] - pair[0]).collect();
        let step = differences.iter().cloned().fold(f64::MAX, f64::min);
        let even = differences.iter().all(|difference| {
            let ratio = difference / step;
            (ratio - ratio.round()).abs() <= 1e-9 * ratio.max(1.0)
        });
        let (positions, steps, step_size) = if even {
            let positions: Vec<usize> = dates
                .iter()
                .map(|date| ((date - dates[0]) / step).round() as usize)
                .collect();
            (
                positions,
                Steps::Even {
                    start: dates[0],
                    step,
                },
                step,
            )
        } else {
            let months: Vec<(i64, u32, u32)> = dates
                .iter()
                .map(|date| month_of(*date))
                .collect::<Option<_>>()
                .ok_or(EtsError::Number)?;
            let day = months[0].1;
            let month_ends = months.iter().all(|(_, day, last)| day == last);
            if !month_ends && months.iter().any(|(_, other, _)| *other != day) {
                return Err(EtsError::Number);
            }
            let start_month = months[0].0;
            let every = months
                .windows(2)
                .map(|pair| pair[1].0 - pair[0].0)
                .fold(0, gcd);
            if every <= 0 {
                return Err(EtsError::Number);
            }
            let positions: Vec<usize> = months
                .iter()
                .map(|(month, _, _)| ((month - start_month) / every) as usize)
                .collect();
            // Excel reports the longest step in days.
            let step_size = dates
                .windows(2)
                .zip(positions.windows(2))
                .map(|(pair, at)| (pair[1] - pair[0]) / (at[1] - at[0]) as f64)
                .fold(0.0, f64::max)
                .round();
            (
                positions,
                Steps::Months {
                    start_month,
                    start_day: if month_ends { 31 } else { day },
                    months: every,
                },
                step_size,
            )
        };
        let length = positions[positions.len() - 1] + 1;
        if length > 1_000_000 {
            return Err(EtsError::Number);
        }
        let mut regular: Vec<Option<f64>> = vec![None; length];
        for (position, value) in positions.iter().zip(aggregated) {
            regular[*position] = value;
        }
        let known = regular.iter().filter(|value| value.is_some()).count();
        if known < 2 {
            return Err(EtsError::Number);
        }
        let mut filled = Vec::with_capacity(length);
        for position in 0..length {
            let value = match regular[position] {
                Some(value) => value,
                None if !complete => 0.0,
                None => {
                    // Linearly between the known neighbours; past the ends,
                    // the nearest one.
                    let before = (0..position)
                        .rev()
                        .find_map(|at| regular[at].map(|v| (at, v)));
                    let after = (position + 1..length).find_map(|at| regular[at].map(|v| (at, v)));
                    match (before, after) {
                        (Some((b, vb)), Some((a, va))) => {
                            vb + (va - vb) * (position - b) as f64 / (a - b) as f64
                        }
                        (Some((_, v)), None) | (None, Some((_, v))) => v,
                        (None, None) => 0.0,
                    }
                }
            };
            filled.push(value);
        }
        Ok(Series {
            values: filled,
            steps,
            step_size,
        })
    }

    /// Where `date` falls on the series, in steps from its first point.
    pub(crate) fn position(&self, date: f64) -> Option<f64> {
        match &self.steps {
            Steps::Even { start, step } => Some((date - start) / step),
            Steps::Months {
                start_month,
                start_day,
                months,
            } => {
                let whole = date.floor();
                let (month, day, last) = month_of(whole)?;
                let anchor = (*start_day).min(last) as f64;
                let offset = (day as f64 + (date - whole) - anchor) / last as f64;
                Some(((month - start_month) as f64 + offset) / *months as f64)
            }
        }
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

/// Autocorrelation of `values` at lags 0 to `lags`.
fn autocorrelation(values: &[f64], lags: usize) -> Vec<f64> {
    let average = mean(values);
    let centred: Vec<f64> = values.iter().map(|v| v - average).collect();
    let variance: f64 = centred.iter().map(|v| v * v).sum();
    if variance <= 0.0 {
        return Vec::new();
    }
    (0..=lags)
        .map(|lag| {
            centred
                .iter()
                .zip(&centred[lag..])
                .map(|(a, b)| a * b)
                .sum::<f64>()
                / variance
        })
        .collect()
}

/// The slope and intercept of the least-squares line through the series.
fn regression(values: &[f64]) -> (f64, f64) {
    let n = values.len() as f64;
    let centre = (n - 1.0) / 2.0;
    let average = mean(values);
    let mut covariance = 0.0;
    let mut spread = 0.0;
    for (index, value) in values.iter().enumerate() {
        let x = index as f64 - centre;
        covariance += x * (value - average);
        spread += x * x;
    }
    let slope = if spread > 0.0 {
        covariance / spread
    } else {
        0.0
    };
    (slope, average - slope * centre)
}

/// The season length a series repeats with, 0 when it has none: the lag
/// at which its deviation from a straight line correlates best with itself,
/// when that is a peak, clearly positive and fits twice with a point over.
pub(crate) fn detect_seasonality(values: &[f64]) -> usize {
    let n = values.len();
    if n < 5 {
        return 0;
    }
    let (slope, intercept) = regression(values);
    let residuals: Vec<f64> = values
        .iter()
        .enumerate()
        .map(|(index, value)| value - (intercept + slope * index as f64))
        .collect();
    let correlation = autocorrelation(&residuals, (n - 1) / 2);
    let mut best = 0;
    let mut best_value = 0.2;
    for lag in 2..correlation.len() {
        if n < 2 * lag + 1 {
            break;
        }
        let value = correlation[lag];
        if value < correlation[lag - 1]
            || correlation.get(lag + 1).is_some_and(|next| value < *next)
        {
            continue;
        }
        if value > best_value {
            best = lag;
            best_value = value;
        }
    }
    best
}

/// A series smoothed with its chosen parameters.
#[derive(Debug, Clone)]
pub(crate) struct Smoothed {
    pub(crate) alpha: f64,
    pub(crate) beta: f64,
    pub(crate) gamma: f64,
    pub(crate) season: usize,
    level: f64,
    trend: f64,
    seasonal: Vec<f64>,
    /// The one-step forecast of each point.
    fitted: Vec<f64>,
    values: Vec<f64>,
}

struct Start {
    level: f64,
    trend: f64,
    seasonal: Vec<f64>,
}

fn start(values: &[f64], season: usize) -> Start {
    let (slope, intercept) = regression(values);
    if season < 2 {
        return Start {
            level: values[0] - slope,
            trend: slope,
            seasonal: Vec::new(),
        };
    }
    let cycles = values.len() / season;
    let means: Vec<f64> = (0..cycles)
        .map(|cycle| mean(&values[cycle * season..(cycle + 1) * season]))
        .collect();
    let trend = if cycles >= 2 {
        (means[1] - means[0]) / season as f64
    } else {
        slope
    };
    let seasonal = if cycles >= 1 {
        (0..season)
            .map(|at| {
                (0..cycles)
                    .map(|cycle| values[cycle * season + at] - means[cycle])
                    .sum::<f64>()
                    / cycles as f64
            })
            .collect()
    } else {
        vec![0.0; season]
    };
    let first = if cycles >= 1 {
        means[0]
    } else {
        intercept + slope * (season as f64 - 1.0) / 2.0
    };
    Start {
        level: first - trend * (season as f64 + 1.0) / 2.0,
        trend,
        seasonal,
    }
}

/// Runs the smoothing over the series; the squared one-step errors' sum,
/// and with `keep` the final state and each point's forecast.
fn run(
    values: &[f64],
    season: usize,
    (alpha, beta, gamma): (f64, f64, f64),
    initial: &Start,
    fitted: Option<&mut Vec<f64>>,
) -> (f64, f64, f64, Vec<f64>) {
    let mut level = initial.level;
    let mut trend = initial.trend;
    let mut seasonal = initial.seasonal.clone();
    let mut squares = 0.0;
    let mut fits = fitted;
    for (index, value) in values.iter().enumerate() {
        let index_in_season = if season >= 2 { index % season } else { 0 };
        let deviation = if season >= 2 {
            seasonal[index_in_season]
        } else {
            0.0
        };
        let forecast = level + trend + deviation;
        if let Some(fits) = fits.as_deref_mut() {
            fits.push(forecast);
        }
        let error = value - forecast;
        squares += error * error;
        let next_level = alpha * (value - deviation) + (1.0 - alpha) * (level + trend);
        let next_trend = beta * (next_level - level) + (1.0 - beta) * trend;
        if season >= 2 {
            seasonal[index_in_season] = gamma * (value - level - trend) + (1.0 - gamma) * deviation;
        }
        level = next_level;
        trend = next_trend;
    }
    (squares, level, trend, seasonal)
}

const LOWEST: f64 = 0.001;
const HIGHEST: f64 = 0.999;

/// Smooths `values` with `season` points a season (0 or 1 for none),
/// choosing α, β and γ.
pub(crate) fn smooth(values: &[f64], season: usize) -> Smoothed {
    let season = if season >= 2 && season < values.len() {
        season
    } else {
        0
    };
    let initial = start(values, season);
    let error = |parameters: (f64, f64, f64)| run(values, season, parameters, &initial, None).0;
    // A coarse grid, then halving steps around the best point.
    let grid = [LOWEST, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, HIGHEST];
    let gammas: &[f64] = if season >= 2 { &grid } else { &[0.0] };
    let mut best = (f64::MAX, [LOWEST, LOWEST, 0.0]);
    for alpha in grid {
        for beta in grid {
            for gamma in gammas {
                let value = error((alpha, beta, *gamma));
                if value < best.0 {
                    best = (value, [alpha, beta, *gamma]);
                }
            }
        }
    }
    let dimensions = if season >= 2 { 3 } else { 2 };
    let mut step = 0.05;
    while step > 1e-4 {
        let mut improved = true;
        while improved {
            improved = false;
            for dimension in 0..dimensions {
                for direction in [-step, step] {
                    let mut candidate = best.1;
                    candidate[dimension] =
                        (candidate[dimension] + direction).clamp(LOWEST, HIGHEST);
                    let value = error((candidate[0], candidate[1], candidate[2]));
                    if value < best.0 - 1e-12 * best.0.abs().max(1e-300) {
                        best = (value, candidate);
                        improved = true;
                    }
                }
            }
        }
        step /= 2.0;
    }
    let [alpha, beta, gamma] = best.1;
    let mut fitted = Vec::with_capacity(values.len());
    let (_, level, trend, seasonal) = run(
        values,
        season,
        (alpha, beta, gamma),
        &initial,
        Some(&mut fitted),
    );
    Smoothed {
        alpha,
        beta,
        gamma: if season >= 2 { gamma } else { f64::EPSILON },
        season,
        level,
        trend,
        seasonal,
        fitted,
        values: values.to_vec(),
    }
}

impl Smoothed {
    /// The forecast `ahead` whole steps past the last point.
    fn ahead(&self, ahead: usize) -> f64 {
        if ahead == 0 {
            return self.fitted[self.fitted.len() - 1];
        }
        let deviation = if self.season >= 2 {
            self.seasonal[(self.values.len() - 1 + ahead) % self.season]
        } else {
            0.0
        };
        self.level + ahead as f64 * self.trend + deviation
    }

    /// The value at `position` steps from the first point: the one-step
    /// forecast within the series, the smoothed forecast past it, linearly
    /// between whole steps.
    pub(crate) fn forecast(&self, position: f64) -> f64 {
        let last = (self.values.len() - 1) as f64;
        let at = |whole: f64| {
            if whole <= last {
                self.fitted[whole.max(0.0) as usize]
            } else {
                self.ahead((whole - last) as usize)
            }
        };
        let below = position.floor();
        let fraction = position - below;
        if fraction == 0.0 {
            return at(below);
        }
        at(below) * (1.0 - fraction) + at(below + 1.0) * fraction
    }

    fn errors(&self) -> impl Iterator<Item = f64> + '_ {
        self.values.iter().zip(&self.fitted).map(|(v, f)| v - f)
    }

    pub(crate) fn mae(&self) -> f64 {
        self.errors().map(f64::abs).sum::<f64>() / self.values.len() as f64
    }

    pub(crate) fn rmse(&self) -> f64 {
        (self.errors().map(|e| e * e).sum::<f64>() / self.values.len() as f64).sqrt()
    }

    pub(crate) fn smape(&self) -> f64 {
        self.values
            .iter()
            .zip(&self.fitted)
            .map(|(value, fit)| {
                let scale = (value.abs() + fit.abs()) / 2.0;
                if scale == 0.0 {
                    0.0
                } else {
                    (value - fit).abs() / scale
                }
            })
            .sum::<f64>()
            / self.values.len() as f64
    }

    /// The mean absolute error over that of the naive forecast, the last
    /// point's value.
    pub(crate) fn mase(&self) -> f64 {
        let naive = self
            .values
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).abs())
            .sum::<f64>()
            / (self.values.len() - 1) as f64;
        if naive == 0.0 {
            0.0
        } else {
            self.mae() / naive
        }
    }

    /// Half the width of the interval the value at `position` falls in
    /// with probability `z`'s: the one-step error's spread grown by how
    /// the smoothing carries errors forward.
    pub(crate) fn interval(&self, position: f64, z: f64) -> f64 {
        let last = (self.values.len() - 1) as f64;
        let ahead = (position - last).ceil().max(1.0) as usize;
        let mut variance = 1.0;
        for step in 1..ahead {
            let seasonal = if self.season >= 2 && step.is_multiple_of(self.season) {
                self.gamma
            } else {
                0.0
            };
            let carried = self.alpha * (1.0 + step as f64 * self.beta) + seasonal;
            variance += carried * carried;
        }
        z * self.rmse() * variance.sqrt()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn some(values: &[f64]) -> Vec<Option<f64>> {
        values.iter().map(|v| Some(*v)).collect()
    }

    #[test]
    fn a_gap_is_completed_and_dates_sorted() {
        let series = Series::new(
            &some(&[3.0, 1.0, 7.0]),
            &some(&[3.0, 1.0, 7.0]),
            true,
            Aggregation::Average,
        )
        .unwrap();
        // Steps of 2 from 1: 1, 3, 5 (missing), 7.
        assert_eq!(series.values, vec![1.0, 3.0, 5.0, 7.0]);
        assert_eq!(series.step_size, 2.0);
        let zeros = Series::new(
            &some(&[3.0, 1.0, 7.0]),
            &some(&[3.0, 1.0, 7.0]),
            false,
            Aggregation::Average,
        )
        .unwrap();
        assert_eq!(zeros.values, vec![1.0, 3.0, 0.0, 7.0]);
    }

    #[test]
    fn duplicates_are_aggregated() {
        let values = some(&[1.0, 3.0, 5.0, 6.0]);
        let timeline = some(&[1.0, 2.0, 2.0, 3.0]);
        let sum = Series::new(&values, &timeline, true, Aggregation::Sum).unwrap();
        assert_eq!(sum.values, vec![1.0, 8.0, 6.0]);
        let max = Series::new(&values, &timeline, true, Aggregation::Max).unwrap();
        assert_eq!(max.values, vec![1.0, 5.0, 6.0]);
    }

    #[test]
    fn an_uneven_timeline_is_refused() {
        let error = Series::new(
            &some(&[1.0, 2.0, 3.0]),
            &some(&[1.0, 2.0, 3.5]),
            true,
            Aggregation::Average,
        );
        assert_eq!(error, Err(EtsError::Number));
        let size = Series::new(
            &some(&[1.0, 2.0]),
            &some(&[1.0, 2.0, 3.0]),
            true,
            Aggregation::Average,
        );
        assert_eq!(size, Err(EtsError::Size));
    }

    #[test]
    fn monthly_dates_step_by_month() {
        // 1 Jan, 1 Feb, 1 Mar 2024 and (missing April) 1 May.
        let timeline = some(&[45292.0, 45323.0, 45352.0, 45413.0]);
        let series = Series::new(
            &some(&[10.0, 20.0, 30.0, 50.0]),
            &timeline,
            true,
            Aggregation::Average,
        )
        .unwrap();
        assert_eq!(series.values, vec![10.0, 20.0, 30.0, 40.0, 50.0]);
        assert_eq!(series.step_size, 31.0);
        // 1 June 2024.
        assert_eq!(series.position(45444.0), Some(5.0));
    }

    #[test]
    fn a_repeating_pattern_is_detected() {
        let values: Vec<f64> = (0..24)
            .map(|i| 10.0 + 0.5 * i as f64 + [3.0, -1.0, -4.0, 2.0][i % 4])
            .collect();
        assert_eq!(detect_seasonality(&values), 4);
        let line: Vec<f64> = (0..10).map(|i| 2.0 * i as f64 + 1.0).collect();
        assert_eq!(detect_seasonality(&line), 0);
    }

    #[test]
    fn a_line_is_continued_exactly() {
        let line: Vec<f64> = (1..=10).map(|i| 2.0 * i as f64 + 1.0).collect();
        let smoothed = smooth(&line, 0);
        assert!((smoothed.forecast(10.0) - 23.0).abs() < 1e-9);
        assert!((smoothed.forecast(21.0) - 45.0).abs() < 1e-9);
        assert!(smoothed.rmse() < 1e-9);
    }
}
