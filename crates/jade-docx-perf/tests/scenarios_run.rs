//! The perf workflow only reports a scenario that runs. A scenario broken by an
//! API change would otherwise surface as a missing row on the next pull
//! request instead of a failure on this one.

#[test]
fn every_scenario_runs_on_a_fixture() {
    let fixture = jade_docx_perf::repo_root().join("public/fixtures/contract.docx");
    for scenario in jade_docx_perf::SCENARIOS {
        jade_docx_perf::run(scenario, &fixture).unwrap_or_else(|err| panic!("{scenario}: {err}"));
    }
}

#[test]
fn an_unknown_scenario_is_an_error() {
    let fixture = jade_docx_perf::repo_root().join("public/fixtures/contract.docx");
    assert!(jade_docx_perf::run("nope", &fixture).is_err());
}
