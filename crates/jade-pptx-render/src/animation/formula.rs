//! The formula language of `<p:anim>` values and `fmla` (ECMA-376
//! 19.5.90): numbers, `#ppt_x`-style shape properties, `$` (the
//! interpolated value), `+ - * / ^`, parentheses and the usual functions.

/// What a formula can read.
#[derive(Debug, Clone, Copy, Default)]
pub struct FormulaScope {
    /// `#ppt_x`, `#ppt_y`: the shape's centre as a fraction of the slide.
    pub x: f64,
    pub y: f64,
    /// `#ppt_w`, `#ppt_h`: its size as a fraction of the slide.
    pub w: f64,
    pub h: f64,
    /// `#ppt_r`: its rotation, degrees.
    pub r: f64,
    /// `$`: the value being shaped.
    pub value: f64,
}

/// Evaluates `formula`; `None` when it does not parse.
pub fn evaluate(formula: &str, scope: &FormulaScope) -> Option<f64> {
    let tokens = tokenize(formula)?;
    let mut parser = Parser {
        tokens: &tokens,
        at: 0,
        scope,
    };
    let value = parser.expression()?;
    (parser.at == tokens.len()).then_some(value)
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Op(char),
}

fn tokenize(input: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // An exponent: 4.16667E-6.
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let save = i;
                i += 1;
                if i < chars.len() && (chars[i] == '-' || chars[i] == '+') {
                    i += 1;
                }
                if i < chars.len() && chars[i].is_ascii_digit() {
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                } else {
                    i = save;
                }
            }
            let text: String = chars[start..i].iter().collect();
            tokens.push(Token::Number(text.parse().ok()?));
        } else if c.is_alphabetic() || c == '#' || c == '_' || c == '$' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            tokens.push(Token::Name(chars[start..i].iter().collect()));
        } else if "+-*/^(),".contains(c) {
            tokens.push(Token::Op(c));
            i += 1;
        } else {
            return None;
        }
    }
    Some(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    at: usize,
    scope: &'a FormulaScope,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, op: char) -> bool {
        if self.peek() == Some(&Token::Op(op)) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expression(&mut self) -> Option<f64> {
        let mut value = self.term()?;
        loop {
            if self.eat('+') {
                value += self.term()?;
            } else if self.eat('-') {
                value -= self.term()?;
            } else {
                return Some(value);
            }
        }
    }

    fn term(&mut self) -> Option<f64> {
        let mut value = self.power()?;
        loop {
            if self.eat('*') {
                value *= self.power()?;
            } else if self.eat('/') {
                let divisor = self.power()?;
                value = if divisor == 0.0 { 0.0 } else { value / divisor };
            } else {
                return Some(value);
            }
        }
    }

    fn power(&mut self) -> Option<f64> {
        let base = self.unary()?;
        if self.eat('^') {
            let exponent = self.power()?;
            Some(base.powf(exponent))
        } else {
            Some(base)
        }
    }

    fn unary(&mut self) -> Option<f64> {
        if self.eat('-') {
            Some(-self.unary()?)
        } else if self.eat('+') {
            self.unary()
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Option<f64> {
        match self.peek()?.clone() {
            Token::Number(value) => {
                self.at += 1;
                Some(value)
            }
            Token::Op('(') => {
                self.at += 1;
                let value = self.expression()?;
                self.eat(')').then_some(value)
            }
            Token::Name(name) => {
                self.at += 1;
                if self.eat('(') {
                    let mut args = vec![self.expression()?];
                    while self.eat(',') {
                        args.push(self.expression()?);
                    }
                    if !self.eat(')') {
                        return None;
                    }
                    call(&name, &args)
                } else {
                    self.variable(&name)
                }
            }
            Token::Op(_) => None,
        }
    }

    fn variable(&self, name: &str) -> Option<f64> {
        let scope = self.scope;
        Some(match name {
            "#ppt_x" | "ppt_x" => scope.x,
            "#ppt_y" | "ppt_y" => scope.y,
            "#ppt_w" | "ppt_w" => scope.w,
            "#ppt_h" | "ppt_h" => scope.h,
            "#ppt_r" | "ppt_r" | "#r" => scope.r,
            "$" => scope.value,
            "pi" => std::f64::consts::PI,
            "e" => std::f64::consts::E,
            _ => return None,
        })
    }
}

fn call(name: &str, args: &[f64]) -> Option<f64> {
    let a = args.first().copied()?;
    Some(match (name, args.len()) {
        ("abs", 1) => a.abs(),
        ("acos", 1) => a.clamp(-1.0, 1.0).acos(),
        ("asin", 1) => a.clamp(-1.0, 1.0).asin(),
        ("atan", 1) => a.atan(),
        ("ceil", 1) => a.ceil(),
        ("cos", 1) => a.cos(),
        ("deg", 1) => a.to_degrees(),
        ("exp", 1) => a.exp(),
        ("floor", 1) => a.floor(),
        ("ln", 1) => {
            if a > 0.0 {
                a.ln()
            } else {
                0.0
            }
        }
        ("max", 2) => a.max(args[1]),
        ("min", 2) => a.min(args[1]),
        ("rad", 1) => a.to_radians(),
        // `rand(x)`: a value in 0..x; a fixed point keeps frames steady.
        ("rand", 1) => a * 0.5,
        ("sin", 1) => a.sin(),
        ("sqrt", 1) => a.max(0.0).sqrt(),
        ("tan", 1) => a.tan(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluates_powerpoint_formulas() {
        let scope = FormulaScope {
            x: 0.5,
            y: 0.25,
            w: 0.2,
            h: 0.1,
            r: 0.0,
            value: 0.5,
        };
        let eval = |f: &str| evaluate(f, &scope).unwrap();
        assert_eq!(eval("#ppt_x"), 0.5);
        assert!((eval("1+#ppt_h/2") - 1.05).abs() < 1e-12);
        assert!((eval("0-#ppt_w/2") + 0.1).abs() < 1e-12);
        assert!((eval("#ppt_y+.1") - 0.35).abs() < 1e-12);
        assert!(
            (eval("#ppt_w*sin(2.5*pi*$)") - 0.2 * (1.25 * std::f64::consts::PI).sin()).abs()
                < 1e-12
        );
        assert!((eval("-4.16667E-6") + 4.16667e-6).abs() < 1e-15);
        assert_eq!(eval("2^3"), 8.0);
        assert!(evaluate("#nope", &scope).is_none());
    }
}
