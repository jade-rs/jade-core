//! 差分エンジン各種 (pptx / pdf / text) で共有する高速化用の下ごしらえ。
//!
//! 対応付けの本体は `diff-align` にある。ここに置くのは、その結果を変えずに
//! 速くするためのもの。
//!
//! - [`TextProfile`]: [`diff_align::similarity`] を何度も引く文字列を先に
//!   bigram にしておく。総当たりで同じ文字列を何度も分解しなくて済む。
//! - [`trimmed_lcs_diff`]: 共通の前後を削ってから、残りを Myers 法で比べる。
//!   [`diff_align::lcs_diff`] と同じ長さの共通部分を返しつつ、長い文字列の
//!   一部だけを変えたときに表が長さの 2 乗に膨らまない。

use diff_align::{lcs_diff, Step};

/// [`diff_align::similarity`] を何度も引く文字列の下ごしらえ。
///
/// 整列済みの文字 bigram と、空白を除いた文字列の長さ・ハッシュを持つ。
/// `TextProfile::new(a).similarity(&TextProfile::new(b))` は
/// `similarity(a, b)` と一致する。
///
/// 文字列そのものは持たない (ページ全文を何百枚分も抱えると重いため)。
/// 完全一致の判定は長さ・ハッシュ・bigram の一致で代える。別の文字列で
/// 3 つとも揃うには、bigram の多重集合が一致したうえで 64 bit のハッシュが
/// 衝突する必要があり、実用上は起きない。
#[derive(Debug, Clone)]
pub struct TextProfile {
    len: usize,
    hash: u64,
    bigrams: Vec<(char, char)>,
}

impl TextProfile {
    pub fn new(text: &str) -> Self {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut len = 0usize;
        let mut previous: Option<char> = None;
        let mut bigrams: Vec<(char, char)> = Vec::new();
        for c in text.chars().filter(|c| !c.is_whitespace()) {
            c.hash(&mut hasher);
            len += 1;
            if let Some(before) = previous {
                bigrams.push((before, c));
            }
            previous = Some(c);
        }
        bigrams.sort_unstable();
        Self {
            len,
            hash: hasher.finish(),
            bigrams,
        }
    }

    fn same_text(&self, other: &Self) -> bool {
        self.len == other.len && self.hash == other.hash && self.bigrams == other.bigrams
    }

    /// 文字 bigram の Dice 係数。0.0 (無関係) 〜 1.0 (一致)。
    pub fn similarity(&self, other: &Self) -> f64 {
        let (a, b) = (self.len, other.len);
        if a == 0 && b == 0 {
            return 1.0;
        }
        if a == 0 || b == 0 {
            return 0.0;
        }
        if self.same_text(other) {
            return 1.0;
        }
        if a == 1 || b == 1 {
            return 0.0;
        }
        let (left, right) = (&self.bigrams, &other.bigrams);
        let (mut i, mut j, mut common) = (0usize, 0usize, 0usize);
        while i < left.len() && j < right.len() {
            match left[i].cmp(&right[j]) {
                std::cmp::Ordering::Equal => {
                    common += 1;
                    i += 1;
                    j += 1;
                }
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
            }
        }
        2.0 * common as f64 / (left.len() + right.len()) as f64
    }

    /// [`Self::similarity`] の上限。長さだけで決まるので、bigram を
    /// 突き合わせる前に見込みの無い組を捨てられる。
    pub fn similarity_upper_bound(&self, other: &Self) -> f64 {
        let (a, b) = (self.bigrams.len(), other.bigrams.len());
        if a == 0 || b == 0 {
            // 1 文字以下は完全一致かどうかだけなので、見積もらずに本計算へ回す。
            return 1.0;
        }
        2.0 * a.min(b) as f64 / (a + b) as f64
    }

    /// `threshold` に届かないと長さだけで分かる組は 0 を返す。届きうる組は
    /// [`Self::similarity`] と同じ値を返す。閾値未満を捨てる対応付けに渡す用。
    pub fn similarity_at_least(&self, other: &Self, threshold: f64) -> f64 {
        if self.similarity_upper_bound(other) < threshold {
            return 0.0;
        }
        self.similarity(other)
    }
}

/// LCS の表を取る区間の上限 (要素数の積)。これを超える変更区間は細かく
/// 突き合わせず、まるごと削除 + 追加として扱う。表 1 セルが 8 バイトなので
/// 上限で約 32 MB。ほぼ全文を書き換えた数千文字の段落がここに当たる。
pub const MAX_LCS_CELLS: usize = 4_000_000;

/// Myers 法で探す編集距離の上限。途中経過は編集距離の 2 乗に比例して
/// 溜まるので (上限で約 32 MB)、これを超える区間は LCS の表に回す。
pub const MAX_MYERS_EDITS: usize = 2_000;

/// 前後の共通部分を削ってから LCS を取る。
///
/// [`lcs_diff`] は入力全体の (n+1)×(m+1) 表を取るので、長い段落に 1 文字
/// 足しただけでも表が文字数の 2 乗に膨らむ。実際の変更は局所的なことが
/// 多いので、変わっていない前後を先に落とす。
///
/// 残った区間はまず Myers 法 (編集距離 D に対して O((n+m)·D)) で解く。
/// 段落の頭で 1 文消えて末尾に 1 文増えた、のように前後を削っても区間が
/// 長いまま残る変更でも、編集が少なければ表を取らずに済む。編集が
/// [`MAX_MYERS_EDITS`] を超えるときは LCS の表に回し、表が
/// [`MAX_LCS_CELLS`] も超えるなら、まるごと削除 + 追加にする。
///
/// どの経路でも共通部分の長さは [`lcs_diff`] と同じ (最長) になる。同じ長さの
/// 並べ方が複数あるときに、どれを選ぶかは経路によって違いうる。
pub fn trimmed_lcs_diff<T: PartialEq>(old: &[T], new: &[T]) -> Vec<Step> {
    trimmed_lcs_diff_with(old, new, MAX_MYERS_EDITS)
}

fn trimmed_lcs_diff_with<T: PartialEq>(old: &[T], new: &[T], max_edits: usize) -> Vec<Step> {
    let prefix = old
        .iter()
        .zip(new)
        .take_while(|(before, after)| before == after)
        .count();
    let suffix = old[prefix..]
        .iter()
        .rev()
        .zip(new[prefix..].iter().rev())
        .take_while(|(before, after)| before == after)
        .count();
    let old_middle = &old[prefix..old.len() - suffix];
    let new_middle = &new[prefix..new.len() - suffix];

    let mut out = Vec::with_capacity(old.len().max(new.len()));
    out.extend((0..prefix).map(|index| Step::Equal(index, index)));
    let middle = if old_middle.is_empty() || new_middle.is_empty() {
        None
    } else {
        myers_diff(old_middle, new_middle, max_edits).or_else(|| {
            (old_middle.len().saturating_mul(new_middle.len()) <= MAX_LCS_CELLS)
                .then(|| lcs_diff(old_middle, new_middle))
        })
    };
    match middle {
        Some(steps) => out.extend(steps.into_iter().map(|step| match step {
            Step::Equal(o, n) => Step::Equal(prefix + o, prefix + n),
            Step::Removed(o) => Step::Removed(prefix + o),
            Step::Added(n) => Step::Added(prefix + n),
        })),
        None => {
            out.extend((0..old_middle.len()).map(|index| Step::Removed(prefix + index)));
            out.extend((0..new_middle.len()).map(|index| Step::Added(prefix + index)));
        }
    }
    let (old_tail, new_tail) = (old.len() - suffix, new.len() - suffix);
    out.extend((0..suffix).map(|offset| Step::Equal(old_tail + offset, new_tail + offset)));
    out
}

/// Myers の O((n+m)·D) 差分。編集距離が `max_edits` を超えたら諦めて None。
///
/// 手順を復元するために、各編集距離 d での到達点 (対角線 -d..=d) を残す。
/// 溜まる量は D の 2 乗程度で、入力の長さには比例しない。
fn myers_diff<T: PartialEq>(old: &[T], new: &[T], max_edits: usize) -> Option<Vec<Step>> {
    let (n, m) = (old.len() as isize, new.len() as isize);
    let limit = max_edits.min(old.len() + new.len()) as isize;
    // v[k] = 対角線 k (= x - y) 上で到達した最も遠い x。k は -limit-1..=limit+1。
    let offset = limit + 1;
    let mut v = vec![0isize; (2 * limit + 3) as usize];
    // trace[d] = 編集距離 d を探す直前の v のうち、対角線 -d-1..=d+1 の部分。
    let mut trace: Vec<Vec<isize>> = Vec::new();

    for d in 0..=limit {
        trace.push(v[(offset - d - 1) as usize..=(offset + d + 1) as usize].to_vec());
        for k in (-d..=d).step_by(2) {
            let at = |k: isize| (offset + k) as usize;
            let mut x = if k == -d || (k != d && v[at(k - 1)] < v[at(k + 1)]) {
                v[at(k + 1)]
            } else {
                v[at(k - 1)] + 1
            };
            let mut y = x - k;
            while x < n && y < m && old[x as usize] == new[y as usize] {
                x += 1;
                y += 1;
            }
            v[at(k)] = x;
            if x >= n && y >= m {
                return Some(myers_backtrack(&trace, n, m));
            }
        }
    }
    None
}

fn myers_backtrack(trace: &[Vec<isize>], n: isize, m: isize) -> Vec<Step> {
    let mut steps = Vec::with_capacity((n + m) as usize);
    let (mut x, mut y) = (n, m);
    for (d, v) in trace.iter().enumerate().rev() {
        let d = d as isize;
        let at = |k: isize| v[(k + d + 1) as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && at(k - 1) < at(k + 1)) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = at(prev_k);
        let prev_y = prev_x - prev_k;
        while x > prev_x && y > prev_y {
            x -= 1;
            y -= 1;
            steps.push(Step::Equal(x as usize, y as usize));
        }
        if d > 0 {
            if x == prev_x {
                steps.push(Step::Added(prev_y as usize));
            } else {
                steps.push(Step::Removed(prev_x as usize));
            }
        }
        x = prev_x;
        y = prev_y;
    }
    steps.reverse();
    steps
}

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn text() -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop::sample::select(vec![
                "a", "b", "日", "本", " ", "\u{0301}", "👩", "\u{200d}", "　", "\n",
            ]),
            0..24,
        )
        .prop_map(|parts| parts.concat())
    }

    #[test]
    fn 上限を超える変更区間は削除と追加にまとめる() {
        let old: Vec<u32> = (0..2_001).collect();
        let new: Vec<u32> = (10_000..12_001).collect();
        let steps = trimmed_lcs_diff(&old, &new);
        assert_eq!(steps.len(), old.len() + new.len());
        assert!(steps[..old.len()]
            .iter()
            .enumerate()
            .all(|(index, step)| *step == Step::Removed(index)));
        assert!(steps[old.len()..]
            .iter()
            .enumerate()
            .all(|(index, step)| *step == Step::Added(index)));
    }

    #[test]
    fn 長い列の一部の変更は前後を削って細かく比べる() {
        let old: Vec<u32> = (0..100_000).collect();
        let mut new = old.clone();
        new.insert(50_000, 999_999);
        let steps = trimmed_lcs_diff(&old, &new);
        let changed: Vec<_> = steps
            .iter()
            .filter(|step| !matches!(step, Step::Equal(..)))
            .collect();
        assert_eq!(changed, vec![&Step::Added(50_000)]);
    }

    proptest! {
        #[test]
        fn 下ごしらえした類似度は元の類似度と一致する(
            old in text(),
            new in text(),
            threshold in 0.0f64..1.0,
        ) {
            let expected = diff_align::similarity(&old, &new);
            let old_profile = TextProfile::new(&old);
            let new_profile = TextProfile::new(&new);
            prop_assert_eq!(old_profile.similarity(&new_profile), expected);
            prop_assert!(old_profile.similarity_upper_bound(&new_profile) >= expected);
            let at_least = old_profile.similarity_at_least(&new_profile, threshold);
            if expected >= threshold {
                prop_assert_eq!(at_least, expected);
            } else {
                prop_assert!(at_least < threshold);
            }
        }

        #[test]
        fn 前後を削った_LCS_は元の_LCS_と同じ長さで両側を覆う(
            old in prop::collection::vec(0u8..4, 0..40),
            new in prop::collection::vec(0u8..4, 0..40),
            // 0 と 1 は Myers をすぐ諦めて LCS の表に回る経路を通す。
            max_edits in prop::sample::select(vec![0usize, 1, 3, MAX_MYERS_EDITS]),
        ) {
            let steps = trimmed_lcs_diff_with(&old, &new, max_edits);
            assert_valid_lcs(&old, &new, &steps)?;
        }

        #[test]
        fn Myers_法は編集距離の上限内なら最長の共通部分を返す(
            old in prop::collection::vec(0u8..3, 0..30),
            new in prop::collection::vec(0u8..3, 0..30),
        ) {
            let steps = myers_diff(&old, &new, usize::MAX).expect("上限なしなら必ず解ける");
            assert_valid_lcs(&old, &new, &steps)?;
            // 編集距離ちょうどの上限でも解け、1 つ少ないと諦める。
            let edits = steps.iter().filter(|step| !matches!(step, Step::Equal(..))).count();
            prop_assert!(myers_diff(&old, &new, edits).is_some());
            if edits > 0 {
                prop_assert!(myers_diff(&old, &new, edits - 1).is_none());
            }
        }
    }

    /// 手順が両側を順に覆い、共通部分が [`lcs_diff`] と同じ長さであること。
    fn assert_valid_lcs(old: &[u8], new: &[u8], steps: &[Step]) -> Result<(), TestCaseError> {
        let equal = |steps: &[Step]| {
            steps
                .iter()
                .filter(|step| matches!(step, Step::Equal(..)))
                .count()
        };
        prop_assert_eq!(equal(steps), equal(&lcs_diff(old, new)));
        let (mut old_at, mut new_at) = (0usize, 0usize);
        for step in steps {
            match *step {
                Step::Equal(o, n) => {
                    prop_assert_eq!((o, n), (old_at, new_at));
                    prop_assert_eq!(old[o], new[n]);
                    old_at += 1;
                    new_at += 1;
                }
                Step::Removed(o) => {
                    prop_assert_eq!(o, old_at);
                    old_at += 1;
                }
                Step::Added(n) => {
                    prop_assert_eq!(n, new_at);
                    new_at += 1;
                }
            }
        }
        prop_assert_eq!((old_at, new_at), (old.len(), new.len()));
        Ok(())
    }

    #[test]
    fn 段落の頭で消えて末尾に増えた文は_Myers_法で細かく比べる() {
        let old: Vec<u32> = (0..3_000).collect();
        let mut new: Vec<u32> = (30..3_000).collect();
        new.extend(10_000..10_030);
        // 前後は削れないので区間は 3000 x 3000 (LCS の表の上限を超える) のまま。
        let steps = trimmed_lcs_diff(&old, &new);
        let removed = steps
            .iter()
            .filter(|step| matches!(step, Step::Removed(_)))
            .count();
        let added = steps
            .iter()
            .filter(|step| matches!(step, Step::Added(_)))
            .count();
        assert_eq!((removed, added), (30, 30));
    }
}
