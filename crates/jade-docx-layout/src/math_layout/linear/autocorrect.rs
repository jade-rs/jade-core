//! Math AutoCorrect: the control words Word replaces as you type in an
//! equation (`\alpha` → α, `\sum` → ∑), which are also UnicodeMath's
//! keywords. The list follows Word's built-in Math AutoCorrect entries
//! (File > Options > Proofing > AutoCorrect Options > Math AutoCorrect).

/// Word's Math AutoCorrect entries: control word (without the backslash)
/// and what it becomes.
pub const MATH_AUTOCORRECT: &[(&str, &str)] = &[
    ("above", "┴"),
    ("acute", "\u{0301}"),
    ("aleph", "ℵ"),
    ("alpha", "α"),
    ("Alpha", "Α"),
    ("amalg", "∐"),
    ("angle", "∠"),
    ("aoint", "∳"),
    ("approx", "≈"),
    ("asmash", "⬆"),
    ("ast", "∗"),
    ("asymp", "≍"),
    ("atop", "¦"),
    ("Bar", "\u{033F}"),
    ("bar", "\u{0305}"),
    ("because", "∵"),
    ("begin", "〖"),
    ("below", "┬"),
    ("bet", "ℶ"),
    ("beta", "β"),
    ("Beta", "Β"),
    ("beth", "ℶ"),
    ("bigcap", "⋂"),
    ("bigcup", "⋃"),
    ("bigodot", "⨀"),
    ("bigoplus", "⨁"),
    ("bigotimes", "⨂"),
    ("bigsqcup", "⨆"),
    ("biguplus", "⨄"),
    ("bigvee", "⋁"),
    ("bigwedge", "⋀"),
    ("binomial", "(a+b)^n=∑_(k=0)^n▒(n¦k) a^k b^(n−k)"),
    ("bot", "⊥"),
    ("bowtie", "⋈"),
    ("box", "□"),
    ("boxdot", "⊡"),
    ("boxminus", "⊟"),
    ("boxplus", "⊞"),
    ("bra", "⟨"),
    ("break", "⤶"),
    ("breve", "\u{0306}"),
    ("bullet", "∙"),
    ("cap", "∩"),
    ("cbrt", "∛"),
    ("cases", "Ⓒ"),
    ("cdot", "⋅"),
    ("cdots", "⋯"),
    ("check", "\u{030C}"),
    ("chi", "χ"),
    ("Chi", "Χ"),
    ("circ", "∘"),
    ("close", "┤"),
    ("clubsuit", "♣"),
    ("coint", "∲"),
    ("cong", "≅"),
    ("coprod", "∐"),
    ("cup", "∪"),
    ("dalet", "ℸ"),
    ("daleth", "ℸ"),
    ("dashv", "⊣"),
    ("dd", "ⅆ"),
    ("Dd", "ⅅ"),
    ("ddddot", "\u{20DC}"),
    ("dddot", "\u{20DB}"),
    ("ddot", "\u{0308}"),
    ("ddots", "⋱"),
    ("defeq", "≝"),
    ("degc", "℃"),
    ("degf", "℉"),
    ("degree", "°"),
    ("delta", "δ"),
    ("Delta", "Δ"),
    ("Deltaeq", "≜"),
    ("diamond", "⋄"),
    ("diamondsuit", "♢"),
    ("div", "÷"),
    ("dot", "\u{0307}"),
    ("doteq", "≐"),
    ("dots", "…"),
    ("downarrow", "↓"),
    ("Downarrow", "⇓"),
    ("dsmash", "⬇"),
    ("ee", "ⅇ"),
    ("ell", "ℓ"),
    ("emptyset", "∅"),
    ("emsp", "\u{2003}"),
    ("end", "〗"),
    ("ensp", "\u{2002}"),
    ("epsilon", "ϵ"),
    ("Epsilon", "Ε"),
    ("eqarray", "█"),
    ("equiv", "≡"),
    ("eta", "η"),
    ("Eta", "Η"),
    ("exists", "∃"),
    ("forall", "∀"),
    ("funcapply", "\u{2061}"),
    ("gamma", "γ"),
    ("Gamma", "Γ"),
    ("ge", "≥"),
    ("geq", "≥"),
    ("gets", "←"),
    ("gg", "≫"),
    ("gimel", "ℷ"),
    ("grave", "\u{0300}"),
    ("hairsp", "\u{200A}"),
    ("hat", "\u{0302}"),
    ("hbar", "ℏ"),
    ("heartsuit", "♡"),
    ("hookleftarrow", "↩"),
    ("hookrightarrow", "↪"),
    ("hphantom", "⬄"),
    ("hsmash", "⬌"),
    ("hvec", "\u{20D1}"),
    ("ii", "ⅈ"),
    ("iiiint", "⨌"),
    ("iiint", "∭"),
    ("iint", "∬"),
    ("Im", "ℑ"),
    ("imath", "ı"),
    ("in", "∈"),
    ("inc", "∆"),
    ("infty", "∞"),
    ("int", "∫"),
    ("integral", "1/2π ∫_0^2π▒ⅆθ/(a+b sin⁡θ)=1/√(a^2−b^2)"),
    ("iota", "ι"),
    ("Iota", "Ι"),
    ("itimes", "\u{2062}"),
    ("jj", "ⅉ"),
    ("jmath", "ȷ"),
    ("kappa", "κ"),
    ("Kappa", "Κ"),
    ("ket", "⟩"),
    ("lambda", "λ"),
    ("Lambda", "Λ"),
    ("langle", "⟨"),
    ("lbbrack", "⟦"),
    ("lbrace", "{"),
    ("lbrack", "["),
    ("lceil", "⌈"),
    ("ldiv", "∕"),
    ("ldivide", "∕"),
    ("ldots", "…"),
    ("le", "≤"),
    ("left", "├"),
    ("leftarrow", "←"),
    ("Leftarrow", "⇐"),
    ("leftharpoondown", "↽"),
    ("leftharpoonup", "↼"),
    ("leftrightarrow", "↔"),
    ("Leftrightarrow", "⇔"),
    ("leq", "≤"),
    ("lfloor", "⌊"),
    ("lhvec", "\u{20D0}"),
    ("limit", "lim_(n→∞)⁡(1+1/n)^n=e"),
    ("ll", "≪"),
    ("lmoust", "⎰"),
    ("Longleftarrow", "⟸"),
    ("Longleftrightarrow", "⟺"),
    ("Longrightarrow", "⟹"),
    ("lrhar", "⇋"),
    ("lvec", "\u{20D6}"),
    ("mapsto", "↦"),
    ("matrix", "■"),
    ("medsp", "\u{205F}"),
    ("mid", "∣"),
    ("middle", "ⓜ"),
    ("models", "⊨"),
    ("mp", "∓"),
    ("mu", "μ"),
    ("Mu", "Μ"),
    ("nabla", "∇"),
    ("naryand", "▒"),
    ("nbsp", "\u{00A0}"),
    ("ne", "≠"),
    ("nearrow", "↗"),
    ("neq", "≠"),
    ("ni", "∋"),
    ("norm", "‖"),
    ("notcontain", "∌"),
    ("notelement", "∉"),
    ("notin", "∉"),
    ("nu", "ν"),
    ("Nu", "Ν"),
    ("nwarrow", "↖"),
    ("o", "ο"),
    ("O", "Ο"),
    ("odot", "⊙"),
    ("of", "▒"),
    ("oiiint", "∰"),
    ("oiint", "∯"),
    ("oint", "∮"),
    ("omega", "ω"),
    ("Omega", "Ω"),
    ("ominus", "⊖"),
    ("open", "├"),
    ("oplus", "⊕"),
    ("otimes", "⊗"),
    ("over", "/"),
    ("overbar", "¯"),
    ("overbrace", "⏞"),
    ("overbracket", "⎴"),
    ("overline", "¯"),
    ("overparen", "⏜"),
    ("overshell", "⏠"),
    ("parallel", "∥"),
    ("partial", "∂"),
    ("perp", "⊥"),
    ("phantom", "⟡"),
    ("phi", "ϕ"),
    ("Phi", "Φ"),
    ("pi", "π"),
    ("Pi", "Π"),
    ("pm", "±"),
    ("pmatrix", "⒨"),
    ("pppprime", "⁗"),
    ("ppprime", "‴"),
    ("pprime", "″"),
    ("prec", "≺"),
    ("preceq", "≼"),
    ("prime", "′"),
    ("prod", "∏"),
    ("propto", "∝"),
    ("psi", "ψ"),
    ("Psi", "Ψ"),
    ("qdrt", "∜"),
    ("quadratic", "x=(−b±√(b^2−4ac))/2a"),
    ("rangle", "⟩"),
    ("Rangle", "⟫"),
    ("ratio", "∶"),
    ("rbrace", "}"),
    ("rbrack", "]"),
    ("Rbrack", "⟧"),
    ("rceil", "⌉"),
    ("rddots", "⋰"),
    ("Re", "ℜ"),
    ("rect", "▭"),
    ("rfloor", "⌋"),
    ("rho", "ρ"),
    ("Rho", "Ρ"),
    ("rhvec", "\u{20D1}"),
    ("right", "┤"),
    ("rightarrow", "→"),
    ("Rightarrow", "⇒"),
    ("rightharpoondown", "⇁"),
    ("rightharpoonup", "⇀"),
    ("rmoust", "⎱"),
    ("root", "⒭"),
    ("sdiv", "⁄"),
    ("sdivide", "⁄"),
    ("searrow", "↘"),
    ("setminus", "∖"),
    ("sigma", "σ"),
    ("Sigma", "Σ"),
    ("sim", "∼"),
    ("simeq", "≃"),
    ("smash", "⬍"),
    ("smile", "⌣"),
    ("spadesuit", "♠"),
    ("sqcap", "⊓"),
    ("sqcup", "⊔"),
    ("sqrt", "√"),
    ("sqsubseteq", "⊑"),
    ("sqsuperseteq", "⊒"),
    ("star", "⋆"),
    ("subset", "⊂"),
    ("subseteq", "⊆"),
    ("succ", "≻"),
    ("succeq", "≽"),
    ("sum", "∑"),
    ("superset", "⊃"),
    ("superseteq", "⊇"),
    ("swarrow", "↙"),
    ("tau", "τ"),
    ("Tau", "Τ"),
    ("therefore", "∴"),
    ("theta", "θ"),
    ("Theta", "Θ"),
    ("thicksp", "\u{2005}"),
    ("thinsp", "\u{2006}"),
    ("tilde", "\u{0303}"),
    ("times", "×"),
    ("to", "→"),
    ("top", "⊤"),
    ("tvec", "\u{20E1}"),
    ("ubar", "\u{0332}"),
    ("Ubar", "\u{0333}"),
    ("underbar", "▁"),
    ("underbrace", "⏟"),
    ("underbracket", "⎵"),
    ("underline", "▁"),
    ("underparen", "⏝"),
    ("uparrow", "↑"),
    ("Uparrow", "⇑"),
    ("updownarrow", "↕"),
    ("Updownarrow", "⇕"),
    ("uplus", "⊎"),
    ("upsilon", "υ"),
    ("Upsilon", "Υ"),
    ("varepsilon", "ε"),
    ("varphi", "φ"),
    ("varpi", "ϖ"),
    ("varrho", "ϱ"),
    ("varsigma", "ς"),
    ("vartheta", "ϑ"),
    ("vbar", "│"),
    ("vdash", "⊢"),
    ("vdots", "⋮"),
    ("vec", "\u{20D7}"),
    ("vee", "∨"),
    ("vert", "|"),
    ("Vert", "‖"),
    ("Vmatrix", "⒩"),
    ("vphantom", "⇳"),
    ("vthicksp", "\u{2004}"),
    ("wedge", "∧"),
    ("wp", "℘"),
    ("wr", "≀"),
    ("xi", "ξ"),
    ("Xi", "Ξ"),
    ("zeta", "ζ"),
    ("Zeta", "Ζ"),
    ("zwnj", "\u{200C}"),
    ("zwsp", "\u{200B}"),
];

/// Word's operator shortcuts typed in an equation: `!=` becomes ≠.
pub const MATH_SEQUENCES: &[(&str, &str)] = &[
    ("<->", "↔"),
    ("<=", "≤"),
    (">=", "≥"),
    ("!=", "≠"),
    ("+-", "±"),
    ("-+", "∓"),
    ("->", "→"),
    ("<-", "←"),
    ("::", "∷"),
    (":=", "≔"),
    ("~=", "≅"),
    ("<<", "≪"),
    (">>", "≫"),
];

/// What a control word (without its backslash) becomes, if Word knows it.
/// Exact matches win; `\Alpha` and `\alpha` differ.
pub fn math_autocorrect(word: &str) -> Option<&'static str> {
    MATH_AUTOCORRECT
        .iter()
        .find(|(name, _)| *name == word)
        .map(|(_, value)| *value)
}

/// Replaces every known `\word` in `text`, and the operator shortcuts, as
/// Word does when a linear-format equation is built up.
pub fn apply_math_autocorrect(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' {
            let start = i + 1;
            let mut end = start;
            while end < chars.len() && chars[end].is_ascii_alphabetic() {
                end += 1;
            }
            if end > start {
                let word: String = chars[start..end].iter().collect();
                if let Some(value) = math_autocorrect(&word) {
                    out.push_str(value);
                    i = end;
                    // The space that ends a control word is consumed.
                    if i < chars.len() && chars[i] == ' ' {
                        i += 1;
                    }
                    continue;
                }
            }
        }
        if let Some((sequence, value)) = MATH_SEQUENCES.iter().find(|(sequence, _)| {
            let len = sequence.chars().count();
            i + len <= chars.len() && chars[i..i + len].iter().copied().eq(sequence.chars())
        }) {
            out.push_str(value);
            i += sequence.chars().count();
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_words_and_shortcuts_become_symbols() {
        assert_eq!(math_autocorrect("alpha"), Some("α"));
        assert_eq!(math_autocorrect("Alpha"), Some("Α"));
        assert_eq!(math_autocorrect("nosuch"), None);
        // The space that ends a control word goes with it.
        assert_eq!(apply_math_autocorrect(r"\alpha +\beta != \infty"), "α+β≠ ∞");
        assert_eq!(apply_math_autocorrect("a<=b->c"), "a≤b→c");
        assert_eq!(apply_math_autocorrect(r"\unknown x"), r"\unknown x");
    }

    #[test]
    fn the_list_is_sorted_and_unique() {
        let mut names: Vec<_> = MATH_AUTOCORRECT.iter().map(|(n, _)| *n).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count);
        assert!(count > 250);
    }
}
