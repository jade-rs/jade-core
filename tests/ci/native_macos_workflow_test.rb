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

  def test_mac_coordinate_gate_is_unconditional_and_uses_locked_dependencies
    step = @steps.find { |item| item["name"] == "Verify unadjusted Word for Mac probe coordinates" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "--locked --test word_mac_coordinates"
  end

  private

  def run_command(step_name)
    @steps.find { |step| step["name"] == step_name }&.fetch("run") ||
      flunk("missing #{step_name.inspect} step")
  end
end
