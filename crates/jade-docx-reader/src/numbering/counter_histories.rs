use super::{
    AbstractNum, LevelOverride, NumberingCounter, NumberingCounterState, NumberingInstance,
    NumberingLevel, NumberingPart,
};
use proptest::prelude::*;

const NUMS: [&str; 5] = ["a", "b", "番号🧪", "", "missing"];
const DEFINITIONS: [&str; 3] = ["shared", "別の定義", ""];
const DEFINITION_FOR_NUM: [usize; 4] = [0, 0, 1, 2];
const LEVELS: usize = 9;

#[derive(Clone, Debug)]
struct Level {
    start: Option<u32>,
    restart: Option<u32>,
}

#[derive(Clone, Debug)]
struct Config {
    definitions: Vec<Option<Vec<Option<Level>>>>,
    overrides: Vec<Vec<Option<u32>>>,
}

impl Config {
    fn level(&self, num: usize, level: usize) -> Option<&Level> {
        self.definitions
            .get(*DEFINITION_FOR_NUM.get(num)?)?
            .as_ref()?
            .get(level)?
            .as_ref()
    }

    fn override_start(&self, num: usize, level: usize) -> Option<u32> {
        self.overrides.get(num)?.get(level).copied().flatten()
    }

    fn start(&self, num: usize, level: usize) -> Option<u32> {
        self.override_start(num, level)
            .or_else(|| self.level(num, level)?.start)
    }

    fn part(&self) -> NumberingPart {
        // Construct only immutable input data here. Counter expectations below
        // read Config directly, without querying these production definitions.
        let mut part = NumberingPart::new();
        for (index, definition) in self.definitions.iter().enumerate() {
            let Some(levels) = definition else { continue };
            part.abstract_nums.insert(
                DEFINITIONS[index].to_owned(),
                AbstractNum {
                    id: DEFINITIONS[index].to_owned(),
                    preset: None,
                    levels: levels
                        .iter()
                        .enumerate()
                        .filter_map(|(index, level)| {
                            let level = level.as_ref()?;
                            Some(NumberingLevel {
                                ilvl: index as u32,
                                start: level.start,
                                restart: level.restart,
                                num_fmt: None,
                                lvl_text: None,
                                lvl_jc: None,
                                suffix: None,
                                indent: None,
                                paragraph_style: None,
                                paragraph_props: None,
                                run_props: None,
                                language: None,
                                picture_bullet: None,
                                is_legal: false,
                            })
                        })
                        .collect(),
                },
            );
        }
        for (num, &definition) in DEFINITION_FOR_NUM.iter().enumerate() {
            part.numbering_instances.insert(
                NUMS[num].to_owned(),
                NumberingInstance {
                    id: NUMS[num].to_owned(),
                    abstract_num_id: DEFINITIONS[definition].to_owned(),
                    overrides: self.overrides[num]
                        .iter()
                        .enumerate()
                        .filter_map(|(level, &start)| {
                            start.map(|_| (level as u32, LevelOverride { start, level: None }))
                        })
                        .collect(),
                },
            );
        }
        part
    }
}

#[derive(Clone, Debug)]
enum Edit {
    Configured {
        num: usize,
        level: usize,
    },
    Advance {
        num: usize,
        definition: usize,
        level: usize,
        start: Option<u32>,
        override_start: bool,
        configured: bool,
    },
    Seed {
        num: usize,
        level: usize,
        configured: bool,
    },
    Save,
    Restore,
    Reset,
}

// The model uses fixed instance/definition slots. It has no production counter,
// map entries, or numbering lookup helpers, and retains unused versus zero.
#[derive(Clone, Debug, Default)]
struct Model {
    concrete: [Vec<Option<u32>>; 5],
    shared: [Vec<Option<u32>>; 3],
}

impl Model {
    #[allow(clippy::too_many_arguments)]
    fn advance(
        &mut self,
        config: Option<&Config>,
        num: usize,
        definition: usize,
        level: usize,
        start: Option<u32>,
        override_start: bool,
    ) -> u32 {
        let mut previous = self.concrete[num].get(level).copied().flatten();
        if !override_start && level < self.shared[definition].len() {
            previous = self.shared[definition][level];
        }
        let value = match previous {
            None => start.unwrap_or(1),
            Some(value) => value + 1,
        };
        for levels in [&mut self.concrete[num], &mut self.shared[definition]] {
            if levels.len() <= level {
                levels.resize(level + 1, None);
            }
            levels[level] = Some(value);
            for (deeper, value) in levels.iter_mut().enumerate().skip(level + 1) {
                let restart = config
                    .and_then(|config| config.level(num, deeper))
                    .and_then(|level| level.restart);
                if restart.is_none_or(|restart| restart as usize > level) {
                    *value = None;
                }
            }
        }
        if let Some(config) = config {
            for ancestor in 0..level.min(self.concrete[num].len()) {
                let shared = self.shared[definition][ancestor];
                if shared.is_some() && config.override_start(num, ancestor).is_none() {
                    self.concrete[num][ancestor] = shared;
                }
            }
        }
        value
    }

    fn seed(&mut self, config: Option<&Config>, num: usize, level: usize) {
        if (0..level).all(|ancestor| {
            self.concrete[num]
                .get(ancestor)
                .is_some_and(Option::is_some)
        }) {
            return;
        }
        let Some(config) = config else { return };
        let Some(&definition) = DEFINITION_FOR_NUM.get(num) else {
            return;
        };
        // Missing abstract definitions do not create either counter vector.
        for ancestor in 0..level {
            if config.level(num, ancestor).is_none() {
                continue;
            }
            self.concrete[num].resize(self.concrete[num].len().max(ancestor + 1), None);
            self.shared[definition].resize(self.shared[definition].len().max(ancestor + 1), None);
            if self.concrete[num][ancestor].is_some() {
                continue;
            }
            if let Some(start) = config.override_start(num, ancestor) {
                self.concrete[num][ancestor] = Some(start);
            } else {
                let value = self.shared[definition][ancestor]
                    .unwrap_or_else(|| config.start(num, ancestor).unwrap_or(1));
                self.concrete[num][ancestor] = Some(value);
                self.shared[definition][ancestor] = Some(value);
            }
        }
    }

    fn state(&self, num: usize, definition: usize) -> NumberingCounterState {
        NumberingCounterState {
            concrete: self.concrete[num].clone(),
            abstract_levels: self.shared[definition].clone(),
        }
    }
}

fn check(config: Config, edits: &[Edit]) {
    let part = config.part();
    let mut counter = NumberingCounter::new();
    let mut model = Model::default();
    let mut saved: Option<(NumberingCounter, Model)> = None;
    for (step, edit) in edits.iter().enumerate() {
        let before = model.clone();
        match *edit {
            Edit::Configured { num, level } => {
                let definition = DEFINITION_FOR_NUM.get(num).copied().unwrap_or(2);
                let expected = model.advance(
                    Some(&config),
                    num,
                    definition,
                    level,
                    config.start(num, level),
                    config.override_start(num, level).is_some(),
                );
                let actual = counter.increment_with(
                    Some(&part),
                    NUMS[num],
                    DEFINITIONS[definition],
                    level as u32,
                    part.start_value(NUMS[num], level as u32),
                    part.has_start_override(NUMS[num], level as u32),
                );
                assert_eq!(actual, expected, "step {step}: {edit:?}");
            }
            Edit::Advance {
                num,
                definition,
                level,
                start,
                override_start,
                configured,
            } => {
                let expected = model.advance(
                    configured.then_some(&config),
                    num,
                    definition,
                    level,
                    start,
                    override_start,
                );
                let actual = if configured {
                    counter.increment_with(
                        Some(&part),
                        NUMS[num],
                        DEFINITIONS[definition],
                        level as u32,
                        start,
                        override_start,
                    )
                } else {
                    counter.increment(
                        NUMS[num],
                        DEFINITIONS[definition],
                        level as u32,
                        start,
                        override_start,
                    )
                };
                assert_eq!(actual, expected, "step {step}: {edit:?}");
            }
            Edit::Seed {
                num,
                level,
                configured,
            } => {
                model.seed(configured.then_some(&config), num, level);
                counter.initialize_ancestor_levels(
                    configured.then_some(&part),
                    NUMS[num],
                    level as u32,
                );
            }
            Edit::Save => saved = Some((counter.clone(), model.clone())),
            Edit::Restore => {
                if let Some((stored, expected)) = &saved {
                    counter = stored.clone();
                    model = expected.clone();
                }
            }
            Edit::Reset => {
                counter = NumberingCounter::new();
                model = Model::default();
            }
        }
        for (num, id) in NUMS.iter().enumerate() {
            for level in 0..12 {
                let expected = model.concrete[num]
                    .get(level)
                    .copied()
                    .flatten()
                    .unwrap_or(0);
                assert_eq!(
                    counter.get_counter(id, level as u32),
                    expected,
                    "step {step}: {edit:?}, num={id:?}, level={level}"
                );
            }
            for (definition, abstract_id) in DEFINITIONS.iter().enumerate() {
                let state = model.state(num, definition);
                assert_eq!(
                    counter.state_for(id, abstract_id),
                    state,
                    "step {step}: {edit:?}, num={id:?}, definition={abstract_id:?}"
                );
                assert!(counter.matches_state(id, abstract_id, &state));
                let old_state = before.state(num, definition);
                assert_eq!(
                    counter.matches_state(id, abstract_id, &old_state),
                    old_state == state,
                    "stale snapshot at step {step}: {edit:?}"
                );
                if let Some((saved_counter, saved_model)) = &saved {
                    assert_eq!(
                        saved_counter.state_for(id, abstract_id),
                        saved_model.state(num, definition),
                        "saved clone changed at step {step}: {edit:?}"
                    );
                }
            }
        }
    }
}

fn config_strategy() -> impl Strategy<Value = Config> {
    let level = prop::option::of((prop::option::of(0u32..8), prop::option::of(0u32..12)))
        .prop_map(|level| level.map(|(start, restart)| Level { start, restart }));
    (
        prop::collection::vec(prop::option::of(prop::collection::vec(level, LEVELS)), 3),
        prop::collection::vec(prop::collection::vec(prop::option::of(0u32..8), LEVELS), 4),
    )
        .prop_map(|(definitions, overrides)| Config {
            definitions,
            overrides,
        })
}

fn edit_strategy() -> impl Strategy<Value = Edit> {
    prop_oneof![
        5 => (0usize..5, 0usize..11).prop_map(|(num, level)| Edit::Configured { num, level }),
        3 => (0usize..5, 0usize..3, 0usize..11, prop::option::of(0u32..8), any::<bool>(), any::<bool>())
            .prop_map(|(num, definition, level, start, override_start, configured)|
                Edit::Advance { num, definition, level, start, override_start, configured }),
        3 => (0usize..5, 0usize..11, any::<bool>())
            .prop_map(|(num, level, configured)| Edit::Seed { num, level, configured }),
        1 => Just(Edit::Save),
        1 => Just(Edit::Restore),
        1 => Just(Edit::Reset),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]
    #[test]
    fn generated_counter_histories_match_model(
        config in config_strategy(), edits in prop::collection::vec(edit_strategy(), 1..81),
    ) {
        check(config, &edits);
    }
}

#[test]
fn zero_starts_shared_lists_and_restart_boundaries_survive_restore() {
    let config = Config {
        definitions: (0..3)
            .map(|_| {
                Some(
                    (0..LEVELS)
                        .map(|level| {
                            Some(Level {
                                start: Some(0),
                                restart: match level {
                                    1 => Some(0),
                                    2 => Some(1),
                                    _ => None,
                                },
                            })
                        })
                        .collect(),
                )
            })
            .collect(),
        overrides: vec![
            vec![None; LEVELS],
            vec![Some(0); LEVELS],
            vec![None; LEVELS],
            vec![None; LEVELS],
        ],
    };
    let mut edits = vec![Edit::Restore];
    for num in 0..NUMS.len() {
        edits.push(Edit::Seed {
            num,
            level: 8,
            configured: true,
        });
        edits.push(Edit::Save);
        for level in [0, 1, 2, 2, 1, 0, 8, 10, 0, 2] {
            edits.push(Edit::Configured { num, level });
            edits.push(Edit::Configured {
                num: (num + 1) % NUMS.len(),
                level,
            });
        }
        edits.extend([Edit::Restore, Edit::Restore, Edit::Reset, Edit::Restore]);
    }
    check(config, &edits);
}
