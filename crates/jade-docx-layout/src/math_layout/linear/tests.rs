use super::*;
use jade_docx_reader::document::MathElementKind as K;

fn omath(text: &str, format: LinearFormat) -> Vec<MathElement> {
    parse_linear(text, format)
}

fn kinds(elements: &[MathElement]) -> Vec<K> {
    elements.iter().map(|e| e.kind.clone()).collect()
}

fn find(elements: &[MathElement], kind: K) -> &MathElement {
    fn walk<'a>(e: &'a MathElement, kind: &K) -> Option<&'a MathElement> {
        if &e.kind == kind {
            return Some(e);
        }
        e.children.iter().find_map(|c| walk(c, kind))
    }
    elements
        .iter()
        .find_map(|e| walk(e, &kind))
        .unwrap_or_else(|| panic!("no {kind:?}"))
}

fn text(e: &MathElement) -> String {
    let mut out = String::new();
    fn walk(e: &MathElement, out: &mut String) {
        if e.kind == K::Text {
            out.push_str(e.text.as_deref().unwrap_or_default());
        }
        for c in &e.children {
            walk(c, out);
        }
    }
    walk(e, &mut out);
    out
}

fn u(text: &str) -> Vec<MathElement> {
    omath(text, LinearFormat::UnicodeMath)
}

#[test]
fn unicode_math_builds_up_the_quadratic_formula() {
    let e = u("x=(−b±√(b^2−4ac))/2a");
    assert_eq!(kinds(&e), [K::Run, K::Fraction]);
    let f = &e[1];
    let num = f.children.iter().find(|c| c.kind == K::Numerator).unwrap();
    let den = f
        .children
        .iter()
        .find(|c| c.kind == K::Denominator)
        .unwrap();
    assert_eq!(text(den), "2a");
    assert_eq!(kinds(&num.children), [K::Run, K::Radical]);
    let rad = find(&e, K::Radical);
    assert!(rad.children.iter().any(|c| c.local_name() == "radPr"));
    assert_eq!(text(find(&e, K::SuperArgument)), "2");
}

#[test]
fn unicode_math_scripts_operators_and_delimiters() {
    let e = u("∑_(i=1)^n▒i");
    let n = find(&e, K::Nary);
    assert_eq!(text(find(&e, K::SubArgument)), "i=1");
    assert_eq!(text(find(&e, K::SuperArgument)), "n");
    assert_eq!(
        text(n.children.iter().find(|c| c.kind == K::Base).unwrap()),
        "i"
    );
    let e = u("(a+b)^2");
    let sup = find(&e, K::Superscript);
    assert_eq!(
        find(std::slice::from_ref(sup), K::Delimiter).kind,
        K::Delimiter
    );
    let e = u("x_i^2");
    assert_eq!(kinds(&e), [K::SubSup]);
    let e = u("a∕b");
    assert!(find(&e, K::Fraction).children[0].children[0].attributes["val"] == "lin");
    let e = u("(n¦k)");
    assert_eq!(kinds(&e), [K::Delimiter]);
    let e = u("|x|");
    assert_eq!(kinds(&e), [K::Delimiter]);
    let e = u("√(3&x)+∛y");
    assert_eq!(text(find(&e, K::Degree)), "3");
}

#[test]
fn unicode_math_functions_matrices_arrays_and_accents() {
    let e = u("sin x+1");
    assert_eq!(kinds(&e), [K::Function, K::Run]);
    assert_eq!(text(find(&e, K::FunctionName)), "sin");
    let e = u("lim_(n→∞)(1+1/n)^n");
    let f = find(&e, K::Function);
    assert_eq!(
        find(std::slice::from_ref(f), K::LimitLower).kind,
        K::LimitLower
    );
    let e = u("■(a&b@c&d)");
    let m = find(&e, K::Matrix);
    assert_eq!(
        m.children.iter().filter(|c| c.kind == K::MatrixRow).count(),
        2
    );
    let e = u("█(x&=1@y&=2)");
    let a = find(&e, K::EquationArray);
    assert_eq!(text(a), "x&=1y&=2");
    let e = u("x\u{0302}+¯(ab)+⏟(a+b)_k");
    assert_eq!(kinds(&e)[0], K::Accent);
    assert!(e.iter().any(|x| x.kind == K::Bar));
    assert_eq!(find(&e, K::LimitLower).kind, K::LimitLower);
    let e = u(r"\alpha+\beta");
    assert_eq!(text(&e[0]), "α+β");
    let e = u("\"if \"x");
    assert!(e[0].children[0]
        .children
        .iter()
        .any(|p| p.local_name() == "nor"));
}

#[test]
fn unicode_math_writes_back_what_it_reads() {
    for source in [
        "x=(−b±√(b^2−4ac))/2a",
        "∑_(i=1)^n▒i",
        "a^2+b^2=c^2",
        "■(a&b@c&d)",
        "█(x&=1@y&=2)",
        "√(3&x)",
        "e^x=1+x/1!+x^2/2!",
        "(a+b)^2",
        "A=πr^2",
    ] {
        let elements = u(source);
        let written = to_linear(&elements, LinearFormat::UnicodeMath);
        let again = u(&written);
        assert_eq!(
            to_linear(&again, LinearFormat::UnicodeMath),
            written,
            "{source} → {written}"
        );
        assert_eq!(
            text_of_all(&elements),
            text_of_all(&again),
            "{source} → {written}"
        );
    }
    assert_eq!(to_linear(&u("a/b"), LinearFormat::UnicodeMath), "a/b");
    assert_eq!(
        to_linear(&u("(a+b)/c"), LinearFormat::UnicodeMath),
        "(a+b)/c"
    );
}

fn text_of_all(elements: &[MathElement]) -> String {
    elements.iter().map(text).collect()
}

fn l(text: &str) -> Vec<MathElement> {
    omath(text, LinearFormat::LaTeX)
}

#[test]
fn latex_reads_the_common_math_subset() {
    let e = l(r"\frac{a}{b}+\sqrt[3]{x}");
    assert_eq!(kinds(&e)[0], K::Fraction);
    assert_eq!(text(find(&e, K::Degree)), "3");
    let e = l(r"x_{i}^{2}");
    assert_eq!(kinds(&e), [K::SubSup]);
    let e = l(r"\sum_{i=1}^{n} i^2");
    let n = find(&e, K::Nary);
    assert_eq!(n.children[0].children[0].attributes["val"], "∑");
    assert_eq!(text(find(&e, K::SubArgument)), "i=1");
    let e = l(r"\left( x+1 \right)");
    assert_eq!(kinds(&e), [K::Delimiter]);
    let e = l(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
    assert_eq!(find(&e, K::Matrix).kind, K::Matrix);
    let e = l(r"\sin x");
    assert_eq!(kinds(&e), [K::Function]);
    let e = l(r"\lim_{n \to \infty} x_n");
    assert_eq!(find(&e, K::LimitLower).kind, K::LimitLower);
    let e = l(r"\hat{x}+\mathbb{R}+\text{if}");
    assert_eq!(kinds(&e)[0], K::Accent);
    assert!(text_of_all(&e).contains('R'));
    let e = l(r"\begin{cases} 1 & x\ge 0 \\ 0 & x<0 \end{cases}");
    assert_eq!(find(&e, K::EquationArray).kind, K::EquationArray);
    let e = l(r"\alpha \leq \beta");
    assert_eq!(text_of_all(&e), "α≤β");
}

#[test]
fn latex_writes_back_what_it_reads() {
    for source in [
        r"\frac{-b\pm \sqrt{b^{2}-4ac}}{2a}",
        r"\sum_{i=1}^{n} i",
        r"\sqrt[3]{x}",
        r"\begin{pmatrix}a & b\\c & d\end{pmatrix}",
        r"\left(a+b\right)^{2}",
        r"\hat{x}",
    ] {
        let elements = l(source);
        let written = to_linear(&elements, LinearFormat::LaTeX);
        let again = l(&written);
        assert_eq!(
            text_of_all(&elements),
            text_of_all(&again),
            "{source} → {written}"
        );
        assert_eq!(kinds(&elements), kinds(&again), "{source} → {written}");
    }
    assert_eq!(
        to_linear(&l(r"\frac{a}{b}"), LinearFormat::LaTeX),
        r"\frac{a}{b}"
    );
}

#[test]
fn either_format_converts_to_the_other() {
    let e = u("x=(−b±√(b^2−4ac))/2a");
    let latex = to_linear(&e, LinearFormat::LaTeX);
    assert!(latex.contains(r"\frac"), "{latex}");
    assert!(latex.contains(r"\sqrt"), "{latex}");
    let back = l(&latex);
    assert_eq!(text_of_all(&back), text_of_all(&e));
}
