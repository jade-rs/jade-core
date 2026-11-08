# frozen_string_literal: true

require "minitest/autorun"
require "yaml"

class NativeMacosWorkflowTest < Minitest::Test
  REPOSITORY_ROOT = File.expand_path("../..", __dir__)
  WORKFLOW_PATH = File.join(REPOSITORY_ROOT, ".github/workflows/ci.yml")
  MANIFESTS = %w[
    crates/jade-skia-painter/Cargo.toml
    crates/jade-docx-native-render/Cargo.toml
    crates/jade-xlsx-native-render/Cargo.toml
  ].freeze

  def setup
    @workflow = YAML.safe_load(File.read(WORKFLOW_PATH), aliases: true)
    @job = @workflow.fetch("jobs").fetch("native-macos")
    @steps = @job.fetch("steps")
  end

  def test_native_job_runs_for_every_pull_request
    triggers = @workflow["on"] || @workflow.fetch(true)
    assert triggers.key?("pull_request")
    refute @job.key?("if"), "native-macos must not be gated by a label or condition"
  end

  def test_native_crates_are_checked_and_linted_on_macos
    assert_equal "macos-latest", @job.fetch("runs-on")

    check_command = run_command("Cargo check native crates")
    clippy_command = run_command("Cargo clippy native crates")

    MANIFESTS.each do |manifest|
      assert_includes check_command, "--manifest-path #{manifest}"
      assert_includes clippy_command, "--manifest-path #{manifest}"
    end
    assert_includes clippy_command, "-- -D warnings"
  end

  def test_native_crates_share_a_cached_target_directory
    assert_equal "${{ github.workspace }}/target/native-macos",
                 @job.fetch("env").fetch("CARGO_TARGET_DIR")

    cache_step = @steps.find { |step| step["uses"]&.start_with?("Swatinem/rust-cache@") }
    refute_nil cache_step, "native-macos must cache Rust build artifacts"

    cached_workspaces = cache_step.fetch("with").fetch("workspaces")
    assert_includes cached_workspaces, ". -> target/native-macos"
  end

  def test_every_native_mac_job_prepares_verified_skia_before_cargo
    %w[word-mac-coordinates native-macos xlsx-native].each do |name|
      job = @workflow.fetch("jobs").fetch(name)
      assert_equal "macos-latest", job.fetch("runs-on")
      steps = job.fetch("steps")
      preparation = steps.index { |step| step["name"] == "Prepare verified macOS Skia binaries" }
      refute_nil preparation, name
      step = steps.fetch(preparation)
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
      assert_includes step.fetch("run"), "scripts/ci/prepare_macos_skia.py"
      assert_includes step.fetch("run"), '--output "$RUNNER_TEMP/'
      first_cargo = steps.index do |item|
        item.fetch("run", "").lines.any? { |line| line.strip.start_with?("cargo ") }
      end
      refute_nil first_cargo, name
      assert_operator preparation, :<, first_cargo, name
    end
  end

  def test_mac_coordinate_gate_is_unconditional_and_uses_locked_dependencies
    step = @steps.find { |item| item["name"] == "Verify unadjusted Word for Mac probe coordinates" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "--locked --test word_mac_coordinates"
  end

  def test_mac_coordinate_comparison_has_an_independent_failing_job
    job = @workflow.fetch("jobs").fetch("word-mac-coordinates")
    assert_equal "macos-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs"), "other jobs must not block the coordinate comparison"
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find do |item|
      item["name"] == "Compare Mac drawing coordinates with committed Word references"
    end
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    command = step.fetch("run")
    assert_includes command, "--locked --test word_mac_coordinates"
    refute_includes command, "--skip"
    refute_includes command, "--ignored"
    refute_includes job.to_s, "JADE_UPDATE_WORD_MAC_SNAPSHOT"
  end

  def test_windows_version_baselines_run_on_windows_without_ignored_failures
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |item| item["name"] == "Compare every registered Word version's baselines" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "--locked --test word_windows_coordinates windows_word_baselines_with_bundled_standins"
    assert_includes step.fetch("run"), "--exact"
    origins = job.fetch("steps").find do |item|
      item["name"] == "Compare Windows Word paragraph and script origins with installed Arial"
    end
    refute_nil origins
    refute origins.key?("if")
    refute origins.fetch("continue-on-error", false)
    assert_includes origins.fetch("run"), "--locked --test word_windows_coordinates windows_word_arial_edge_origins"
    assert_includes origins.fetch("run"), "--exact"
    refute_includes origins.fetch("run"), "--skip"
    refute_includes origins.fetch("run"), "--ignored"
    editing = job.fetch("steps").find do |item|
      item["name"] == "Preserve spaced table geometry through editing and export"
    end
    refute_nil editing
    refute editing.key?("if")
    refute editing.fetch("continue-on-error", false)
    assert_includes editing.fetch("run"), "--locked --test word_windows_coordinates spaced_tables_preserve_origins_after_edit_export_and_undo"
    assert_includes editing.fetch("run"), "--exact"
    refute_includes editing.fetch("run"), "--skip"
    refute_includes editing.fetch("run"), "--ignored"
  end

  def test_windows_editing_and_font_reuse_cannot_silently_stop_running
    job = @workflow.fetch("jobs").fetch("windows-native-editing")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    {
      "Verify Windows font reuse and rendering" => %w[windows_font_reuse font_styles opentype_features],
      "Verify Windows native editing histories" => %w[native_document tables typing_differential]
    }.each do |name, suites|
      step = steps.find { |item| item["name"] == name }
      refute_nil step, "missing #{name}"
      refute step.key?("if")
      refute step.fetch("continue-on-error", false)
      command = step.fetch("run")
      assert_includes command, "--locked"
      assert_includes command, "--test-threads=1"
      suites.each { |suite| assert_includes command, "--test #{suite}" }
      refute_includes command, "--skip"
      refute_includes command, "--ignored"
    end
  end

  def test_windows_vector_border_comparison_is_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    {
      "Install pinned vector border verifier" => "requirements-borders.txt",
      "Verify border displacement and missing-paint rejection cases" => "test_windows_borders.py",
      "Build vector drawing probe" => "--locked --example word_coordinates",
      "Compare Windows table borders with every registered Word version" => "compare_windows_borders.py --index tests/word-oracle/windows/index.json"
    }.each do |name, command|
      step = job.fetch("steps").find { |item| item["name"] == name }
      refute_nil step, "missing #{name}"
      refute step.key?("if")
      refute step.fetch("continue-on-error", false)
      assert_includes step.fetch("run"), command
      refute_includes step.fetch("run"), "--skip"
    end
  end

  private

  def run_command(step_name)
    @steps.find { |step| step["name"] == step_name }&.fetch("run") ||
      flunk("missing #{step_name.inspect} step")
  end
end
