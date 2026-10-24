use super::{bad, Result};
use jade_xlsx_reader::CellValue;
#[derive(Clone, Default)]
pub(super) struct Aggregate {
    count: usize,
    n: usize,
    sum: f64,
    correction: f64,
    mean: f64,
    m2: f64,
    product: f64,
    min: Option<f64>,
    max: Option<f64>,
    error: Option<String>,
}
impl Aggregate {
    pub fn add(&mut self, value: &CellValue) {
        if !matches!(value, CellValue::Empty) {
            self.count += 1;
        }
        match value {
            CellValue::Number(x) => {
                self.n += 1;
                let y = x - self.correction;
                let t = self.sum + y;
                self.correction = (t - self.sum) - y;
                self.sum = t;
                let delta = x - self.mean;
                self.mean += delta / self.n as f64;
                self.m2 += delta * (x - self.mean);
                self.product = if self.n == 1 { *x } else { self.product * x };
                self.min = Some(self.min.map_or(*x, |m| m.min(*x)));
                self.max = Some(self.max.map_or(*x, |m| m.max(*x)));
            }
            CellValue::Error(e) if self.error.is_none() => self.error = Some(e.clone()),
            _ => {}
        }
    }
    pub fn value(&self, kind: &str) -> Result<CellValue> {
        if kind == "count" {
            return Ok(CellValue::Number(self.count as f64));
        }
        if kind == "countNums" {
            return Ok(CellValue::Number(self.n as f64));
        }
        if let Some(e) = &self.error {
            return Ok(CellValue::Error(e.clone()));
        }
        let value = match kind {
            "sum" => self.sum,
            "average" if self.n > 0 => self.mean,
            "average" => return Ok(CellValue::Error("#DIV/0!".into())),
            "min" => self.min.unwrap_or(0.),
            "max" => self.max.unwrap_or(0.),
            "product" => {
                if self.n > 0 {
                    self.product
                } else {
                    0.
                }
            }
            "var" | "stdDev" if self.n < 2 => return Ok(CellValue::Error("#DIV/0!".into())),
            "varp" | "stdDevp" if self.n == 0 => return Ok(CellValue::Error("#DIV/0!".into())),
            "var" => self.m2 / (self.n - 1) as f64,
            "varp" => self.m2 / self.n as f64,
            "stdDev" => (self.m2 / (self.n - 1) as f64).max(0.).sqrt(),
            "stdDevp" => (self.m2 / self.n as f64).max(0.).sqrt(),
            _ => return Err(bad("uninterpreted pivot aggregation")),
        };
        Ok(if value.is_finite() {
            CellValue::Number(value)
        } else {
            CellValue::Error("#NUM!".into())
        })
    }
}
