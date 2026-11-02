use statrs::distribution::{ContinuousCDF, Normal};

use super::ets;
use crate::expressions::types::CellReferenceIndex;
use crate::{
    calc_result::CalcResult, expressions::parser::Node, expressions::token::Error, model::Model,
};

impl<'a> Model<'a> {
    // FORECAST(x, known_y's, known_x's) / FORECAST.LINEAR(x, known_y's, known_x's)
    // Returns the predicted y value for a given x using simple linear regression.
    fn fn_forecast_linear_impl(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        if args.len() != 3 {
            return CalcResult::new_args_number_error(cell);
        }

        let x = match self.get_number(&args[0], cell) {
            Ok(v) => v,
            Err(e) => return e,
        };

        let (_, _, values_y, values_x) = match self.fn_get_two_matrices(&args[1..], cell) {
            Ok(s) => s,
            Err(e) => return e,
        };

        let mut n = 0.0_f64;
        let mut sum_x = 0.0_f64;
        let mut sum_y = 0.0_f64;
        let mut sum_x2 = 0.0_f64;
        let mut sum_xy = 0.0_f64;

        let len = values_y.len().min(values_x.len());
        for i in 0..len {
            if let (Some(y), Some(xi)) = (values_y[i], values_x[i]) {
                n += 1.0;
                sum_x += xi;
                sum_y += y;
                sum_x2 += xi * xi;
                sum_xy += xi * y;
            }
        }

        if n < 2.0 {
            return CalcResult::new_error(
                Error::DIV,
                cell,
                "FORECAST requires at least two numeric data points".to_string(),
            );
        }

        let denom = n * sum_x2 - sum_x * sum_x;
        if denom == 0.0 || !denom.is_finite() {
            return CalcResult::new_error(
                Error::DIV,
                cell,
                "Division by zero in FORECAST: all x values are equal".to_string(),
            );
        }

        let slope = (n * sum_xy - sum_x * sum_y) / denom;
        let intercept = (sum_y - slope * sum_x) / n;
        let result = intercept + slope * x;

        if !result.is_finite() {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "Numerical error in FORECAST".to_string(),
            );
        }

        CalcResult::Number(result)
    }

    pub(crate) fn fn_forecast(&mut self, args: &[Node], cell: CellReferenceIndex) -> CalcResult {
        self.fn_forecast_linear_impl(args, cell)
    }

    pub(crate) fn fn_forecast_linear(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        self.fn_forecast_linear_impl(args, cell)
    }

    /// A vector argument of the ETS functions: each number, `None` for
    /// anything else.
    fn ets_vector(
        &mut self,
        node: &Node,
        cell: CellReferenceIndex,
    ) -> Result<Vec<Option<f64>>, CalcResult> {
        match self.evaluate_node_in_context(node, cell) {
            CalcResult::Range { left, right } => {
                if left.sheet != right.sheet {
                    return Err(CalcResult::new_error(
                        Error::VALUE,
                        cell,
                        "Ranges are in different sheets".to_string(),
                    ));
                }
                self.values_from_range(left, right)
            }
            CalcResult::Array(array) => self
                .values_from_array(array)
                .map_err(|error| CalcResult::new_error(error, cell, "Error in array".to_string())),
            CalcResult::Number(value) => Ok(vec![Some(value)]),
            error @ CalcResult::Error { .. } => Err(error),
            _ => Err(CalcResult::new_error(
                Error::VALUE,
                cell,
                "Expected numbers".to_string(),
            )),
        }
    }

    /// The optional number at `index`, `default` when left out.
    fn ets_option(
        &mut self,
        args: &[Node],
        index: usize,
        default: f64,
        cell: CellReferenceIndex,
    ) -> Result<f64, CalcResult> {
        match args.get(index) {
            None | Some(Node::EmptyArgKind) => Ok(default),
            Some(node) => self.get_number(node, cell),
        }
    }

    /// The series and its smoothing from `values`, `timeline` and the
    /// seasonality, data completion and aggregation arguments at `options`.
    fn ets_smoothed(
        &mut self,
        args: &[Node],
        (values, timeline): (usize, usize),
        options: Option<usize>,
        completion_at: usize,
        cell: CellReferenceIndex,
    ) -> Result<(ets::Series, ets::Smoothed), CalcResult> {
        let number_error =
            |message: &str| CalcResult::new_error(Error::NUM, cell, message.to_string());
        let values = self.ets_vector(&args[values], cell)?;
        let timeline = self.ets_vector(&args[timeline], cell)?;
        let seasonality = match options {
            Some(index) => self.ets_option(args, index, 1.0, cell)?.trunc(),
            None => 1.0,
        };
        let completion = self.ets_option(args, completion_at, 1.0, cell)?.trunc();
        let aggregation = self.ets_option(args, completion_at + 1, 1.0, cell)?;
        if !(0.0..=8760.0).contains(&seasonality) {
            return Err(number_error("Seasonality must be between 0 and 8760"));
        }
        if completion != 0.0 && completion != 1.0 {
            return Err(number_error("Data completion must be 0 or 1"));
        }
        let Some(aggregation) = ets::Aggregation::from_code(aggregation) else {
            return Err(number_error("Aggregation must be between 1 and 7"));
        };
        let series = match ets::Series::new(&values, &timeline, completion == 1.0, aggregation) {
            Ok(series) => series,
            Err(ets::EtsError::Size) => {
                return Err(CalcResult::new_error(
                    Error::NA,
                    cell,
                    "Values and timeline must be the same size".to_string(),
                ))
            }
            Err(ets::EtsError::Number) => {
                return Err(number_error("The timeline needs a constant step"))
            }
        };
        let season = if seasonality == 1.0 {
            ets::detect_seasonality(&series.values)
        } else {
            seasonality as usize
        };
        if season >= 2 && season >= series.values.len() {
            return Err(number_error("Seasonality is longer than the data"));
        }
        let smoothed = ets::smooth(&series.values, season);
        Ok((series, smoothed))
    }

    /// Where `target` falls on the series; before its start is `#NUM!`.
    fn ets_target(
        &mut self,
        node: &Node,
        series: &ets::Series,
        cell: CellReferenceIndex,
    ) -> Result<f64, CalcResult> {
        let target = self.get_number(node, cell)?;
        match series.position(target) {
            Some(position) if position >= 0.0 && position.is_finite() => Ok(position),
            _ => Err(CalcResult::new_error(
                Error::NUM,
                cell,
                "The target date is before the timeline".to_string(),
            )),
        }
    }

    // FORECAST.ETS(target_date, values, timeline, [seasonality], [data_completion], [aggregation])
    pub(crate) fn fn_forecast_ets(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(3..=6).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let (series, smoothed) = match self.ets_smoothed(args, (1, 2), Some(3), 4, cell) {
            Ok(result) => result,
            Err(error) => return error,
        };
        match self.ets_target(&args[0], &series, cell) {
            Ok(position) => CalcResult::Number(smoothed.forecast(position)),
            Err(error) => error,
        }
    }

    // FORECAST.ETS.CONFINT(target_date, values, timeline, [confidence_level], [seasonality],
    //                      [data_completion], [aggregation])
    pub(crate) fn fn_forecast_ets_confint(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(3..=7).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let confidence = match self.ets_option(args, 3, 0.95, cell) {
            Ok(confidence) => confidence,
            Err(error) => return error,
        };
        if confidence <= 0.0 || confidence >= 1.0 {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "The confidence level must be between 0 and 1".to_string(),
            );
        }
        let (series, smoothed) = match self.ets_smoothed(args, (1, 2), Some(4), 5, cell) {
            Ok(result) => result,
            Err(error) => return error,
        };
        let position = match self.ets_target(&args[0], &series, cell) {
            Ok(position) => position,
            Err(error) => return error,
        };
        let z = match Normal::new(0.0, 1.0) {
            Ok(normal) => normal.inverse_cdf((1.0 + confidence) / 2.0),
            Err(_) => return CalcResult::new_error(Error::NUM, cell, "Normal".to_string()),
        };
        CalcResult::Number(smoothed.interval(position, z))
    }

    // FORECAST.ETS.SEASONALITY(values, timeline, [data_completion], [aggregation])
    pub(crate) fn fn_forecast_ets_seasonality(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(2..=4).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        match self.ets_smoothed(args, (0, 1), None, 2, cell) {
            Ok((_, smoothed)) => CalcResult::Number(smoothed.season as f64),
            Err(error) => error,
        }
    }

    // FORECAST.ETS.STAT(values, timeline, statistic_type, [seasonality], [data_completion],
    //                   [aggregation])
    pub(crate) fn fn_forecast_ets_stat(
        &mut self,
        args: &[Node],
        cell: CellReferenceIndex,
    ) -> CalcResult {
        if !(3..=6).contains(&args.len()) {
            return CalcResult::new_args_number_error(cell);
        }
        let statistic = match self.get_number(&args[2], cell) {
            Ok(statistic) => statistic.trunc(),
            Err(error) => return error,
        };
        if !(1.0..=8.0).contains(&statistic) {
            return CalcResult::new_error(
                Error::NUM,
                cell,
                "The statistic type must be between 1 and 8".to_string(),
            );
        }
        let (series, smoothed) = match self.ets_smoothed(args, (0, 1), Some(3), 4, cell) {
            Ok(result) => result,
            Err(error) => return error,
        };
        CalcResult::Number(match statistic as i32 {
            1 => smoothed.alpha,
            2 => smoothed.beta,
            3 => smoothed.gamma,
            4 => smoothed.mase(),
            5 => smoothed.smape(),
            6 => smoothed.mae(),
            7 => smoothed.rmse(),
            _ => series.step_size,
        })
    }
}
