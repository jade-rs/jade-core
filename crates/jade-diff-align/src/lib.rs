//! 対応付けの土台。差分エンジン各種で共有する。
//!
//! スライド / ページ / 図形 / 段落 と、比べる対象は違っても「先に対応付けて
//! から中身を比べる」構造は同じなので、「スコア関数を渡すと 1:1 の対応を
//! 返す」形にまとめてある。移動の検出は、対応が付いたペアのうち「順序を
//! 保ったまま最大数残せる部分列」(LIS) を据え置きとみなし、そこから外れた
//! ものを移動として報告する。
//!
//! 総当たり ([`pair_by_score`]) は要素数の 2 乗に効くので、1000 ページの PDF
//! のような大きな入力には [`pair_with_anchors`] を使う。中身が完全に一致する
//! ものを先に錨として固定し、その隙間だけを総当たりする (patience diff と
//! 同じ考え方)。ほとんどのページが変わっていない実際の文書では、これで
//! 総当たりの対象がごく一部に縮む。

use std::collections::HashMap;
use std::hash::Hash;

/// 対応付けの結果 1 件。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pairing {
    /// 旧 index と新 index が対応した。
    Matched {
        old: usize,
        new: usize,
        moved: bool,
    },
    Removed(usize),
    Added(usize),
}

/// 文字 bigram の Dice 係数。0.0 (無関係) 〜 1.0 (一致)。
///
/// 単語区切りの無い日本語でも効くように、単語ではなく文字の 2-gram を使う。
pub fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().filter(|c| !c.is_whitespace()).collect();
    let b: Vec<char> = b.chars().filter(|c| !c.is_whitespace()).collect();
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    if a.len() == 1 || b.len() == 1 {
        return if a == b { 1.0 } else { 0.0 };
    }

    let bigrams =
        |chars: &[char]| -> Vec<(char, char)> { chars.windows(2).map(|w| (w[0], w[1])).collect() };
    let mut left = bigrams(&a);
    let mut right = bigrams(&b);
    left.sort_unstable();
    right.sort_unstable();

    // ソート済み同士を突き合わせて共通要素を数える (重複も考慮する)。
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

/// スコアが最大のペアから順に確定させる 1:1 対応付け。
///
/// `score` は 0.0〜1.0 を返し、`threshold` 未満は対応なしとみなす。
pub fn pair_by_score<F>(
    old_len: usize,
    new_len: usize,
    threshold: f64,
    mut score: F,
) -> Vec<Pairing>
where
    F: FnMut(usize, usize) -> f64,
{
    let mut candidates: Vec<(f64, usize, usize)> = Vec::new();
    for old in 0..old_len {
        for new in 0..new_len {
            let value = score(old, new);
            if value >= threshold {
                candidates.push((value, old, new));
            }
        }
    }
    // 高スコア順。同点なら添字が近い順にして、結果を決定的にする。
    candidates.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let da = a.1.abs_diff(a.2);
                let db = b.1.abs_diff(b.2);
                da.cmp(&db)
            })
            .then_with(|| a.1.cmp(&b.1))
    });

    let mut old_taken = vec![false; old_len];
    let mut new_taken = vec![false; new_len];
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for (_, old, new) in candidates {
        if old_taken[old] || new_taken[new] {
            continue;
        }
        old_taken[old] = true;
        new_taken[new] = true;
        pairs.push((old, new));
    }
    pairs.sort_by_key(|(old, _)| *old);

    // 順序を保てるペアの最大集合 = 据え置き。残りが移動。
    let kept = longest_increasing_pairs(&pairs);
    let mut out: Vec<Pairing> = Vec::new();
    for (index, (old, new)) in pairs.iter().enumerate() {
        out.push(Pairing::Matched {
            old: *old,
            new: *new,
            moved: !kept.contains(&index),
        });
    }
    for (old, taken) in old_taken.iter().enumerate() {
        if !taken {
            out.push(Pairing::Removed(old));
        }
    }
    for (new, taken) in new_taken.iter().enumerate() {
        if !taken {
            out.push(Pairing::Added(new));
        }
    }
    out
}

/// 錨を打ってから隙間だけ総当たりする対応付け。
///
/// `keys` は「完全に一致していれば同じもの」と言い切れる値 (ページ本文など)。
/// 両側でちょうど 1 回ずつ現れる key を候補にし、順序を保てる最大集合を
/// 錨として固定する。錨に挟まれた区間だけ [`pair_by_score`] に渡すので、
/// 変更が局所的な文書では総当たりがほぼ消える。
///
/// `max_pairwise` は 1 区間あたりに許す総当たりの組数。これを超える区間は
/// スコアを諦めて出現順で対応付ける (大きく食い違う文書での保険)。
pub fn pair_with_anchors<K, F>(
    old_keys: &[K],
    new_keys: &[K],
    threshold: f64,
    max_pairwise: usize,
    mut score: F,
) -> Vec<Pairing>
where
    K: Hash + Eq,
    F: FnMut(usize, usize) -> f64,
{
    let anchors = unique_common_anchors(old_keys, new_keys);
    if anchors.is_empty() {
        // 錨が無い = 共通部分が無いので、まとめて総当たりに回す。
        return bounded_pairs(
            0,
            old_keys.len(),
            0,
            new_keys.len(),
            threshold,
            max_pairwise,
            &mut score,
        );
    }

    let mut out: Vec<Pairing> = Vec::new();
    let (mut old_cursor, mut new_cursor) = (0usize, 0usize);
    for &(old, new) in &anchors {
        out.extend(bounded_pairs(
            old_cursor,
            old,
            new_cursor,
            new,
            threshold,
            max_pairwise,
            &mut score,
        ));
        out.push(Pairing::Matched {
            old,
            new,
            moved: false,
        });
        old_cursor = old + 1;
        new_cursor = new + 1;
    }
    out.extend(bounded_pairs(
        old_cursor,
        old_keys.len(),
        new_cursor,
        new_keys.len(),
        threshold,
        max_pairwise,
        &mut score,
    ));
    out
}

/// 両側でちょうど 1 回ずつ現れる key の対応のうち、順序を保てる最大集合。
fn unique_common_anchors<K: Hash + Eq>(old_keys: &[K], new_keys: &[K]) -> Vec<(usize, usize)> {
    let mut old_positions: HashMap<&K, (usize, usize)> = HashMap::new();
    for (index, key) in old_keys.iter().enumerate() {
        old_positions
            .entry(key)
            .and_modify(|slot| slot.1 += 1)
            .or_insert((index, 1));
    }
    let mut new_positions: HashMap<&K, (usize, usize)> = HashMap::new();
    for (index, key) in new_keys.iter().enumerate() {
        new_positions
            .entry(key)
            .and_modify(|slot| slot.1 += 1)
            .or_insert((index, 1));
    }

    let mut pairs: Vec<(usize, usize)> = old_positions
        .iter()
        .filter(|(_, (_, count))| *count == 1)
        .filter_map(|(key, (old, _))| {
            new_positions
                .get(key)
                .filter(|(_, count)| *count == 1)
                .map(|(new, _)| (*old, *new))
        })
        .collect();
    pairs.sort_unstable();

    let kept = longest_increasing_pairs(&pairs);
    kept.into_iter().map(|index| pairs[index]).collect()
}

/// 区間 [old_start, old_end) と [new_start, new_end) を対応付ける。
fn bounded_pairs<F>(
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    threshold: f64,
    max_pairwise: usize,
    score: &mut F,
) -> Vec<Pairing>
where
    F: FnMut(usize, usize) -> f64,
{
    let old_len = old_end.saturating_sub(old_start);
    let new_len = new_end.saturating_sub(new_start);
    if old_len == 0 && new_len == 0 {
        return Vec::new();
    }
    if old_len == 0 {
        return (new_start..new_end).map(Pairing::Added).collect();
    }
    if new_len == 0 {
        return (old_start..old_end).map(Pairing::Removed).collect();
    }

    if old_len.saturating_mul(new_len) > max_pairwise {
        // 総当たりが重すぎる区間は出現順で対応付ける。
        let common = old_len.min(new_len);
        let mut out: Vec<Pairing> = (0..common)
            .map(|offset| Pairing::Matched {
                old: old_start + offset,
                new: new_start + offset,
                moved: false,
            })
            .collect();
        out.extend((old_start + common..old_end).map(Pairing::Removed));
        out.extend((new_start + common..new_end).map(Pairing::Added));
        return out;
    }

    pair_by_score(old_len, new_len, threshold, |old, new| {
        score(old_start + old, new_start + new)
    })
    .into_iter()
    .map(|pairing| match pairing {
        Pairing::Matched { old, new, moved } => Pairing::Matched {
            old: old_start + old,
            new: new_start + new,
            moved,
        },
        Pairing::Removed(old) => Pairing::Removed(old_start + old),
        Pairing::Added(new) => Pairing::Added(new_start + new),
    })
    .collect()
}

/// 新 index が増加し続ける最長部分列の添字集合 (LIS)。
fn longest_increasing_pairs(pairs: &[(usize, usize)]) -> Vec<usize> {
    if pairs.is_empty() {
        return Vec::new();
    }

    #[derive(Clone, Copy)]
    struct Candidate {
        length: usize,
        index: usize,
    }

    fn better(left: Candidate, right: Candidate) -> Candidate {
        if right.length > left.length || (right.length == left.length && right.index < left.index) {
            right
        } else {
            left
        }
    }

    let missing = Candidate {
        length: 0,
        index: usize::MAX,
    };
    let mut positions: Vec<_> = pairs.iter().map(|pair| pair.1).collect();
    positions.sort_unstable();
    positions.dedup();

    let mut tree = vec![missing; positions.len() + 1];
    let mut prev = vec![usize::MAX; pairs.len()];
    let mut tail = missing;
    for (index, pair) in pairs.iter().enumerate() {
        let rank = positions
            .binary_search(&pair.1)
            .expect("position was collected")
            + 1;
        let mut prefix = rank - 1;
        let mut predecessor = missing;
        while prefix > 0 {
            predecessor = better(predecessor, tree[prefix]);
            prefix &= prefix - 1;
        }

        let candidate = Candidate {
            length: predecessor.length + 1,
            index,
        };
        if predecessor.index != usize::MAX {
            prev[index] = predecessor.index;
        }
        let mut tree_index = rank;
        while tree_index < tree.len() {
            tree[tree_index] = better(tree[tree_index], candidate);
            tree_index += tree_index & tree_index.wrapping_neg();
        }
        tail = better(tail, candidate);
    }

    let mut out = Vec::new();
    let mut cursor = tail.index;
    while cursor != usize::MAX {
        out.push(cursor);
        cursor = prev[cursor];
    }
    out.reverse();
    out
}

/// 逐次比較の 1 手。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Equal(usize, usize),
    Removed(usize),
    Added(usize),
}

/// 並び順を保った差分 (LCS)。段落や文字列の比較に使う。
pub fn lcs_diff<T: PartialEq>(old: &[T], new: &[T]) -> Vec<Step> {
    let (n, m) = (old.len(), new.len());
    // 表は (n+1) x (m+1)。段落・文字数はたかが知れているので素直に取る。
    let mut table = vec![0usize; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[at(i, j)] = if old[i] == new[j] {
                table[at(i + 1, j + 1)] + 1
            } else {
                table[at(i + 1, j)].max(table[at(i, j + 1)])
            };
        }
    }

    let mut out = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(Step::Equal(i, j));
            i += 1;
            j += 1;
        } else if table[at(i + 1, j)] >= table[at(i, j + 1)] {
            out.push(Step::Removed(i));
            i += 1;
        } else {
            out.push(Step::Added(j));
            j += 1;
        }
    }
    while i < n {
        out.push(Step::Removed(i));
        i += 1;
    }
    while j < m {
        out.push(Step::Added(j));
        j += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn quadratic_lis(pairs: &[(usize, usize)]) -> Vec<usize> {
        if pairs.is_empty() {
            return Vec::new();
        }
        let mut best = vec![1usize; pairs.len()];
        let mut prev = vec![usize::MAX; pairs.len()];
        for i in 1..pairs.len() {
            for j in 0..i {
                if pairs[j].1 < pairs[i].1 && best[j] + 1 > best[i] {
                    best[i] = best[j] + 1;
                    prev[i] = j;
                }
            }
        }
        let mut tail = 0;
        for i in 1..pairs.len() {
            if best[i] > best[tail] {
                tail = i;
            }
        }
        let mut out = Vec::new();
        let mut cursor = tail;
        while cursor != usize::MAX {
            out.push(cursor);
            cursor = prev[cursor];
        }
        out.reverse();
        out
    }

    #[test]
    fn similarity_is_one_for_identical_text() {
        assert_eq!(similarity("売上サマリ", "売上サマリ"), 1.0);
    }

    #[test]
    fn similarity_is_between_for_near_text() {
        let score = similarity("東京 120", "東京 150");
        assert!(score > 0.3 && score < 1.0, "unexpected score: {score}");
    }

    #[test]
    fn similarity_is_zero_for_unrelated_text() {
        assert!(similarity("りんご", "決算報告").abs() < f64::EPSILON);
    }

    #[test]
    fn pairs_identical_sequences_in_order() {
        let old = ["a", "b", "c"];
        let new = ["a", "b", "c"];
        let pairs = pair_by_score(3, 3, 0.5, |i, j| similarity(old[i], new[j]));
        assert!(pairs.iter().all(|p| matches!(
            p,
            Pairing::Matched {
                old,
                new,
                moved: false
            } if old == new
        )));
    }

    #[test]
    fn detects_insertion_without_shifting_the_rest() {
        let old = ["intro", "sales", "closing"];
        let new = ["intro", "summary", "sales", "closing"];
        let pairs = pair_by_score(3, 4, 0.5, |i, j| similarity(old[i], new[j]));
        let added: Vec<_> = pairs
            .iter()
            .filter_map(|p| match p {
                Pairing::Added(i) => Some(*i),
                _ => None,
            })
            .collect();
        assert_eq!(added, vec![1]);
        assert!(
            pairs
                .iter()
                .all(|p| !matches!(p, Pairing::Matched { moved: true, .. }))
        );
    }

    #[test]
    fn detects_move() {
        let old = ["a", "b", "c"];
        let new = ["c", "a", "b"];
        let pairs = pair_by_score(3, 3, 0.5, |i, j| similarity(old[i], new[j]));
        let moved: Vec<_> = pairs
            .iter()
            .filter_map(|p| match p {
                Pairing::Matched {
                    old, moved: true, ..
                } => Some(*old),
                _ => None,
            })
            .collect();
        assert_eq!(moved, vec![2], "only the relocated item should be a move");
    }

    #[test]
    fn anchors_pin_identical_items_and_only_score_the_gaps() {
        let old = ["intro", "委託料は 500,000 円", "body", "closing"];
        let new = ["intro", "委託料は 550,000 円", "body", "closing"];
        let mut scored = 0usize;
        let pairs = pair_with_anchors(&old, &new, 0.3, 1_000, |i, j| {
            scored += 1;
            similarity(old[i], new[j])
        });

        // intro / body / closing は錨。総当たりは残った 1 組だけ。
        assert_eq!(scored, 1, "総当たりの回数: {scored}");
        assert!(pairs.contains(&Pairing::Matched {
            old: 1,
            new: 1,
            moved: false
        }));
    }

    #[test]
    fn anchors_handle_insertions_and_deletions() {
        let old = ["a", "b", "c"];
        let new = ["a", "x", "b", "c"];
        let pairs = pair_with_anchors(&old, &new, 0.9, 1_000, |i, j| similarity(old[i], new[j]));
        assert!(pairs.contains(&Pairing::Added(1)));
        assert_eq!(
            pairs
                .iter()
                .filter(|p| matches!(p, Pairing::Matched { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn anchors_fall_back_to_order_when_a_gap_is_too_large() {
        // 共通部分が無く、総当たりの上限も超える → 出現順で対応付ける。
        let old = ["a", "b", "c"];
        let new = ["x", "y", "z"];
        let mut scored = 0usize;
        let pairs = pair_with_anchors(&old, &new, 0.3, 4, |i, j| {
            scored += 1;
            similarity(old[i], new[j])
        });
        assert_eq!(scored, 0, "上限を超えたらスコアは計算しない");
        assert_eq!(
            pairs
                .iter()
                .filter(|p| matches!(p, Pairing::Matched { .. }))
                .count(),
            3
        );
    }

    #[test]
    fn anchors_ignore_repeated_keys() {
        // 同じ内容が両側に何度も出る場合は錨にできない (どれと対応するか決まらない)。
        let old = ["same", "same", "only-old"];
        let new = ["same", "same", "only-new"];
        let pairs = pair_with_anchors(&old, &new, 0.3, 1_000, |i, j| similarity(old[i], new[j]));
        assert_eq!(pairs.len(), 3, "全要素がどれかに分類される: {pairs:?}");
    }

    #[test]
    fn nlogn_lis_matches_quadratic_tie_breaks_for_every_small_permutation() {
        fn visit(values: &mut [usize], start: usize, check: &mut impl FnMut(&[usize])) {
            if start == values.len() {
                check(values);
                return;
            }
            for index in start..values.len() {
                values.swap(start, index);
                visit(values, start + 1, check);
                values.swap(start, index);
            }
        }

        let mut positions: Vec<_> = (0..7).collect();
        visit(&mut positions, 0, &mut |positions| {
            let pairs: Vec<_> = positions.iter().copied().enumerate().collect();
            assert_eq!(
                longest_increasing_pairs(&pairs),
                quadratic_lis(&pairs),
                "new positions: {positions:?}"
            );
        });
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 2_048,
            max_shrink_iters: 10_000,
            ..ProptestConfig::default()
        })]

        #[test]
        fn nlogn_lis_matches_quadratic_for_arbitrary_positions(
            positions in prop::collection::vec(0usize..512, 0..192),
        ) {
            let pairs: Vec<_> = positions.iter().copied().enumerate().collect();
            prop_assert_eq!(longest_increasing_pairs(&pairs), quadratic_lis(&pairs));
        }

        #[test]
        fn anchor_pairing_classifies_each_input_exactly_once(
            old in prop::collection::vec(0u8..32, 0..64),
            new in prop::collection::vec(0u8..32, 0..64),
        ) {
            let pairs = pair_with_anchors(&old, &new, 0.5, 4_096, |i, j| {
                if old[i] == new[j] { 1.0 } else { 0.0 }
            });
            let mut old_counts = vec![0usize; old.len()];
            let mut new_counts = vec![0usize; new.len()];

            for pairing in pairs {
                match pairing {
                    Pairing::Matched { old: old_index, new: new_index, .. } => {
                        prop_assert_eq!(old[old_index], new[new_index]);
                        old_counts[old_index] += 1;
                        new_counts[new_index] += 1;
                    }
                    Pairing::Removed(old_index) => old_counts[old_index] += 1,
                    Pairing::Added(new_index) => new_counts[new_index] += 1,
                }
            }

            prop_assert!(old_counts.iter().all(|count| *count == 1));
            prop_assert!(new_counts.iter().all(|count| *count == 1));
        }

        #[test]
        fn identical_sequences_never_report_changes_or_moves(
            values in prop::collection::vec(0u8..32, 0..128),
        ) {
            let pairs = pair_with_anchors(&values, &values, 0.5, 16_384, |i, j| {
                if values[i] == values[j] { 1.0 } else { 0.0 }
            });
            let expected: Vec<_> = (0..values.len())
                .map(|index| Pairing::Matched {
                    old: index,
                    new: index,
                    moved: false,
                })
                .collect();

            prop_assert_eq!(pairs, expected);
        }
    }

    #[test]
    fn lcs_reports_insertions_and_deletions() {
        let steps = lcs_diff(&[1, 2, 3], &[1, 3, 4]);
        assert_eq!(
            steps,
            vec![
                Step::Equal(0, 0),
                Step::Removed(1),
                Step::Equal(2, 1),
                Step::Added(2),
            ]
        );
    }
}
