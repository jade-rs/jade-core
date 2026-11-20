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

  def test_windows_xps_assertions_run_after_all_required_native_outputs
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |step| step["name"] == "Assert independent versioned Windows Word XPS print origins" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_xps.py --compare-rendered", step.fetch("run")
    %w[check_windows_aptos_sizes.py check_windows_aptos_nominal.py check_windows_positive_kerning.py].each do |script|
      producer = steps.index { |item| item.fetch("run", "").include?(script + " --native-executable") }
      refute_nil producer, script
      assert_operator producer, :<, index, script
    end
    assert steps.any? { |item| item.fetch("run", "").include?("-p test_windows_xps.py") }
  end

  def test_windows_color_span_assertions_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |s| s["name"] == "Assert versioned Windows Word color-span screen and print origins" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_color_spans.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe", step.fetch("run")
    assert job.fetch("steps").any? { |s| s.fetch("run", "").include?("-p test_windows_color_span_probes.py") }
  end

  def test_windows_word_boundary_assertions_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |s| s["name"] == "Assert versioned Windows Word word-boundary screen and print origins" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_word_boundaries.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe", step.fetch("run")
    assert job.fetch("steps").any? { |s| s.fetch("run", "").include?("-p test_windows_word_boundary_probes.py") }
  end

  def test_windows_y_length_assertions_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |s| s["name"] == "Assert versioned Windows Word repeated-y screen and print origins" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_y_lengths.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe", step.fetch("run")
    assert job.fetch("steps").any? { |s| s.fetch("run", "").include?("-p test_windows_y_length_probes.py") }
  end

  def test_windows_y_pdf_context_assertions_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |s| s["name"] == "Assert versioned Windows Word paragraph-order screen and print origins" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_y_pdf_context.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe", step.fetch("run")
    assert job.fetch("steps").any? { |s| s.fetch("run", "").include?("-p test_windows_y_pdf_context_probes.py") }
  end

  def test_windows_device_remainder_assertions_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find { |s| s["name"] == "Assert versioned Windows Word device-remainder screen and print origins" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_device_remainders.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe", step.fetch("run")
    assert job.fetch("steps").any? { |s| s.fetch("run", "").include?("-p test_windows_device_remainder_probes.py") }
  end

  def test_windows_manual_break_assertions_are_required_after_font_rendering
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |s| s["name"] == "Assert versioned Windows Word manual-break XPS origins" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_hard_break_spaces.py --compare-rendered-fonts target/windows-font-coordinates", step.fetch("run")
    producer = steps.index { |s| s["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil producer
    assert_operator producer, :<, index
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_hard_break_space_probes.py") }
  end

  def test_windows_legacy_justification_and_pending_text_are_required_after_font_rendering
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |s| s["name"] == "Assert versioned Windows Word legacy justification and pending text" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_legacy_justification.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe --compare-rendered-fonts target/windows-font-coordinates", step.fetch("run")
    producer = steps.index { |s| s["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil producer
    assert_operator producer, :<, index
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_legacy_justification_probes.py") }
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_xps_fonts.py") }
  end

  def test_versioned_render_artifacts_exclude_decoded_original_font_programs
    step = @workflow.fetch("jobs").fetch("windows-word-baselines").fetch("steps").find do |item|
      item["name"] == "Upload versioned Windows manual-break and legacy observations"
    end
    refute_nil step
    assert_equal "always()", step.fetch("if")
    %w[ttf otf ttc].each do |extension|
      assert_includes step.fetch("with").fetch("path"), "!target/windows-font-coordinates/**/fonts/*.#{extension}"
    end
    assert_includes step.fetch("with").fetch("name"), "16.0.19127.20936"
  end

  def test_windows_space_pair_kerning_checks_are_required_after_font_rendering
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |s| s["name"] == "Assert versioned Windows Word space-pair kerning XPS origins" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_space_pair.py --compare-rendered-fonts target/windows-font-coordinates", step.fetch("run")
    producer = steps.index { |s| s["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil producer
    assert_operator producer, :<, index
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_space_pair_probes.py") }
  end

  def test_windows_mixed_face_justification_checks_are_required_after_font_rendering
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |s| s["name"] == "Assert versioned Windows Word mixed-face justification and source font metrics" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_mixed_face.py --compare-rendered-fonts target/windows-font-coordinates", step.fetch("run")
    producer = steps.index { |s| s["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil producer
    assert_operator producer, :<, index
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_mixed_face_probes.py") }
  end

  def test_windows_font_slot_spacing_checks_are_required_after_font_rendering
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    assert_equal 105, job.fetch("timeout-minutes")
    refute job.key?("if")
    refute job.key?("needs")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    index = steps.index { |s| s["name"] == "Assert versioned Windows Word mixed-script font-slot spacing" }
    refute_nil index
    step = steps.fetch(index)
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_equal "python scripts/word-oracle/check_windows_font_slot_spacing.py --compare-rendered-fonts target/windows-font-coordinates", step.fetch("run")
    producer = steps.index { |s| s["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil producer
    assert_operator producer, :<, index
    assert steps.any? { |s| s.fetch("run", "").include?("-p test_windows_font_slot_spacing_probes.py") }
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
      assert_equal "${{ github.token }}", step.fetch("env").fetch("GH_TOKEN"), name
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

  def test_original_platform_expectations_are_required_in_every_coordinate_job
    %w[word-mac-coordinates word-mac-browser-coordinates windows-word-baselines].each do |name|
      job = @workflow.fetch("jobs").fetch(name)
      assert_equal(name == "windows-word-baselines" ? "windows-latest" : "macos-latest", job.fetch("runs-on"), name)
      refute job.key?("if"), name
      refute job.key?("needs"), name
      refute job.fetch("continue-on-error", false), name
      steps = job.fetch("steps")
      originals = steps.select do |step|
        step.fetch("run", "").include?("scripts/word-oracle/check_platform_coordinates.py")
      end
      assert_equal 1, originals.length, name
      step = originals.fetch(0)
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
      command = step.fetch("run")
      assert_includes command, "-p test_platform_coordinates.py", name
      refute_match(/\|\|\s*(true|echo)|continue-on-error|--skip|--ignored/, command, name)
      if name == "windows-word-baselines"
        assert_includes command, "--native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
      else
        refute_includes command, "--native-executable", name
        renderer = steps.index do |item|
          run = item.fetch("run", "")
          name == "word-mac-coordinates" ? run.include?("--test word_mac_coordinates") :
            run.include?("node scripts/ci/check_mac_word_coordinates.mjs")
        end
        refute_nil renderer, name
        assert_operator steps.index(step), :<=, renderer, name
      end
    end
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

  def test_windows_renderer_artifact_is_bound_before_comparisons
    steps = @workflow.fetch("jobs").fetch("windows-word-baselines").fetch("steps")
    checkout = steps.find { |step| step["name"] == "Checkout repository" }
    assert_operator checkout.fetch("with").fetch("fetch-depth"), :>=, 2
    build = steps.find { |step| step["id"] == "windows-coordinate-renderer" }
    provenance = steps.find { |step| step["id"] == "windows-coordinate-renderer-provenance" }
    upload = steps.find { |step| step["name"] == "Upload verified Windows coordinate renderer" }
    [build, provenance, upload].each { |step| refute_nil step }
    refute provenance.key?("if")
    refute provenance.fetch("continue-on-error", false)
    assert_includes provenance.fetch("run"), "-p test_word_renderer_build.py"
    assert_includes provenance.fetch("run"), "python scripts/ci/record_word_renderer_build.py"
    assert_equal "${{ github.event.pull_request.head.sha || github.sha }}",
                 provenance.fetch("env").fetch("WORD_RENDERER_EXPECTED_HEAD")
    assert_equal "${{ always() && steps.windows-coordinate-renderer-provenance.outcome == 'success' }}", upload.fetch("if")
    assert_equal "error", upload.fetch("with").fetch("if-no-files-found")
    assert_includes upload.fetch("with").fetch("path"), "word_coordinates.exe"
    assert_includes upload.fetch("with").fetch("path"), "word-renderer-build.json"
    assert_operator steps.index(build), :<, steps.index(provenance)
    assert_operator steps.index(provenance), :<, steps.index(upload)
    assert_operator steps.index(upload), :<,
                    steps.index { |step| step["name"] == "Assert native shaping platform boundaries" }
  end

  def test_windows_native_assertions_stop_at_the_first_failed_command
    %w[windows-word-baselines windows-native-editing].each do |name|
      job = @workflow.fetch("jobs").fetch(name)
      assert_equal "bash", job.fetch("defaults").fetch("run").fetch("shell"), name
      job.fetch("steps").select { |step| step.key?("run") }.each do |step|
        assert_equal "bash", step.fetch("shell", "bash"), step.fetch("name", name)
        refute_includes step.fetch("run"), "set +e", name
        refute_includes step.fetch("run"), "|| true", name
      end
    end
  end

  def test_column_table_sources_and_word_output_contrasts_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    step = job.fetch("steps").find { |item| item["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "test_windows_column_table_probes.py"
    gate = job.fetch("steps").find { |item| item["name"] == "Assert Windows table screen, PDF and vector-rule coordinates" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "test_windows_column_table_gate.py"
    assert_includes gate.fetch("run"), "check_windows_column_tables.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
  end

  def test_minimum_line_sources_and_native_coordinates_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    source = job.fetch("steps").find { |item| item["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil source
    refute source.key?("if")
    refute source.fetch("continue-on-error", false)
    assert_includes source.fetch("run"), "test_windows_minimum_line_probes.py"
    gate = job.fetch("steps").find { |item| item["name"] == "Assert Windows minimum-line coordinates and preserve screen geometry" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "test_windows_minimum_line_gate.py"
    assert_includes gate.fetch("run"), "check_windows_minimum_lines.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
  end

  def test_percentage_table_sources_and_native_coordinates_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    source = job.fetch("steps").find { |item| item["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil source
    refute source.key?("if")
    refute source.fetch("continue-on-error", false)
    assert_includes source.fetch("run"), "test_windows_percentage_table_probes.py"
    gate = job.fetch("steps").find { |item| item["name"] == "Assert Windows percentage-table coordinates and preserve screen geometry" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "test_windows_percentage_table_gate.py"
    assert_includes gate.fetch("run"), "check_windows_percentage_tables.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
  end

  def test_aptos_size_control_sources_and_versioned_originals_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    source = job.fetch("steps").find { |item| item["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil source
    refute source.key?("if")
    refute source.fetch("continue-on-error", false)
    assert_includes source.fetch("run"), "test_windows_aptos_size_probes.py"
    assert_includes source.fetch("run"), "test_windows_explicit_hyphen_probes.py"
    gate = job.fetch("steps").find { |item| item["name"] == "Assert Windows Aptos screen origins and validated PDF origins" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "test_windows_aptos_size_gate.py"
    assert_includes gate.fetch("run"), "check_windows_aptos_sizes.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
  end

  def test_windows_positive_kerning_coordinate_assertion_is_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    source = job.fetch("steps").find { |step| step["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil source
    assert_includes source.fetch("run"), "test_windows_positive_kerning_probes.py"
    gate = job.fetch("steps").find { |step| step["name"] == "Assert Windows positive kerning screen and validated PDF origins" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "check_windows_positive_kerning.py --native-executable"
  end

  def test_windows_nominal_coordinate_assertion_is_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    gate = job.fetch("steps").find { |step| step["name"] == "Assert Windows Aptos nominal screen origins and validated PDF origins" }
    refute_nil gate
    refute gate.key?("if")
    refute gate.fetch("continue-on-error", false)
    assert_includes gate.fetch("run"), "check_windows_aptos_nominal.py --native-executable"
  end

  def test_top_bottom_control_sources_and_versioned_originals_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    source = job.fetch("steps").find { |item| item["name"] == "Verify unpromoted original Windows Word observations" }
    refute_nil source
    refute source.key?("if")
    refute source.fetch("continue-on-error", false)
    assert_includes source.fetch("run"), "test_windows_top_bottom_probes.py"
    assert_includes source.fetch("run"), "test_windows_pdf_segment_probes.py"
    assert_includes source.fetch("run"), "test_windows_aptos_nominal_probes.py"
    assert_includes source.fetch("run"), "test_windows_printer_toggle_probes.py"
    refute_includes source.fetch("run"), "|| true"
  end

  def test_windows_arial_and_top_bottom_screen_print_coordinates_are_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    step = job.fetch("steps").find { |item| item["name"] == "Assert Windows Arial and top-bottom geometry against original Word screen and PDF" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "test_windows_top_bottom_clearance.py"
    assert_includes step.fetch("run"), "check_windows_top_bottom_clearance.py --native-executable"
    refute_includes step.fetch("run"), "|| true"
  end

  def test_workspace_and_release_audit_have_bounded_individual_time_budgets
    job = @workflow.fetch("jobs").fetch("test")
    assert_equal 60, job.fetch("timeout-minutes")
    refute job.fetch("continue-on-error", false)
    steps = job.fetch("steps")
    {
      "Install Rust toolchain" => 10,
      "Cargo test" => 45,
      "Audit edited DOCX fixture corpus" => 15
    }.each do |name, minutes|
      step = steps.find { |item| item["name"] == name }
      refute_nil step, name
      assert_equal minutes, step.fetch("timeout-minutes"), name
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
    end
    cache = steps.find { |item| item["uses"]&.start_with?("Swatinem/rust-cache@") }
    refute_nil cache
    assert_equal 10, cache.fetch("timeout-minutes")
  end

  def test_edit_export_and_reopen_audit_is_a_required_current_head_step
    job = @workflow.fetch("jobs").fetch("test")
    refute job.key?("if")
    audit = job.fetch("steps").find { |item| item["name"] == "Audit edited DOCX fixture corpus" }
    refute_nil audit
    refute audit.key?("if")
    refute audit.fetch("continue-on-error", false)
    assert_includes audit.fetch("run"), "python3 scripts/audit-mac-release-corpus.py"
  end

  def test_shared_originals_use_independent_required_platform_comparisons
    {
      "word-mac-browser-coordinates" => [
        "macos-latest", "Compare synthetic DOCX layouts with Word coordinates",
        ["test_platform_coordinates.py", "check_platform_coordinates.py",
         "node scripts/ci/check_mac_word_coordinates.mjs"]
      ],
      "windows-word-baselines" => [
        "windows-latest", "Assert shared Mac originals against Windows Word version expectations",
        ["test_platform_coordinates.py",
         "test_windows_float_lookahead_probes.py",
         "test_windows_calibri_title_tracking_probes.py",
         "test_windows_ms_auto_spacing_probes.py",
         "check_platform_coordinates.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"]
      ]
    }.each do |name, (runner, step_name, commands)|
      job = @workflow.fetch("jobs").fetch(name)
      assert_equal runner, job.fetch("runs-on"), name
      refute job.key?("if"), name
      refute job.key?("needs"), name
      refute job.fetch("continue-on-error", false), name
      step = job.fetch("steps").find { |item| item["name"] == step_name }
      refute_nil step, name
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
      command = step.fetch("run")
      commands.each { |required| assert_includes command, required, name }
      refute_includes command, "|| true", name
      refute_includes command, "--skip", name
      refute_includes command, "--ignored", name
      # The Mac command must enumerate the whole imported corpus. Positional
      # case names would silently stop asserting the other shared originals.
      if name == "word-mac-browser-coordinates"
        assert_includes command.lines.map(&:strip), "node scripts/ci/check_mac_word_coordinates.mjs"
      end
    end
  end

  def test_mac_original_pdf_assertions_install_pinned_word_dependencies
    %w[word-mac-browser-coordinates word-mac-coordinates].each do |name|
      steps = @workflow.fetch("jobs").fetch(name).fetch("steps")
      setup = steps.index do |step|
        step.fetch("uses", "") == "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065"
      end
      refute_nil setup, name
      assert_equal "3.12", steps.fetch(setup).fetch("with").fetch("python-version"), name
      install = steps.index { |step| step["name"] == "Install Word reference verification dependencies" }
      refute_nil install, name
      step = steps.fetch(install)
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
      assert_equal "python -m pip install -r scripts/word-oracle/requirements-borders.txt", step.fetch("run"), name
      comparison = steps.index { |item| item.fetch("run", "").include?("test_platform_coordinates.py") }
      refute_nil comparison, name
      assert_operator setup, :<, install, name
      assert_operator install, :<, comparison, name
    end
    requirements = File.read(File.join(REPOSITORY_ROOT, "scripts/word-oracle/requirements-borders.txt"))
    assert_includes requirements.lines.map(&:strip), "PyMuPDF==2.0"
  end

  def test_numeric_spacing_sources_and_platform_shaping_are_required
    jobs = @workflow.fetch("jobs")
    %w[windows-word-baselines native-macos].each do |name|
      step = jobs.fetch(name).fetch("steps").find { |item| item["name"] == "Assert native shaping platform boundaries" }
      refute_nil step, name
      refute step.key?("if"), name
      refute step.fetch("continue-on-error", false), name
      assert_equal "cargo test --manifest-path crates/jade-docx-layout/Cargo.toml --locked --features native-shaping --lib", step.fetch("run")
    end
    step = jobs.fetch("windows-word-baselines").fetch("steps").find { |item| item["name"] == "Assert Windows font coordinates with original Word embedded fonts" }
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    assert_includes step.fetch("run"), "test_windows_numeric_spacing_probes.py"
    assert_includes step.fetch("run"), "test_windows_courier_grid_probes.py"
    assert_includes step.fetch("run"), "check_windows_font_coordinates.py --native-executable"
  end

  def test_windows_source_form_hyphen_paint_requires_originals_and_rejection_checks
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find do |item|
      item["name"] == "Assert source-form Windows hyphen paint against original Word fonts"
    end
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    command = step.fetch("run")
    assert_includes command, "test_windows_mixed_hyphen_probes.py"
    assert_includes command, "test_windows_hyphen_paint.py"
    assert_includes command, "check_windows_hyphen_paint.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
    refute_includes command, "|| true"
    refute_includes command, "--skip"
    refute_includes command, "--ignored"
  end

  def test_windows_screen_and_pdf_origins_remain_independently_required
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    assert_equal "windows-latest", job.fetch("runs-on")
    refute job.key?("if")
    refute job.fetch("continue-on-error", false)
    step = job.fetch("steps").find do |item|
      item["name"] == "Assert independent Windows Word screen and vector PDF origins"
    end
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    command = step.fetch("run")
    assert_includes command, "test_windows_output_contexts.py"
    assert_includes command, "check_windows_output_contexts.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
    refute_includes command, "|| true"
    refute_includes command, "--skip"
    refute_includes command, "--ignored"
  end

  def test_windows_graphic_text_keeps_required_paint_and_rejection_checks
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    step = job.fetch("steps").find do |item|
      item["name"] == "Assert versioned Windows text effects and absolute outline geometry"
    end
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    command = step.fetch("run")
    assert_includes command, "test_windows_graphic_text.py"
    assert_includes command, "test_windows_actualtext_aliases.py"
    assert_includes command, "check_windows_graphic_text.py --native-executable target/windows-word-baselines/debug/examples/word_coordinates.exe"
    refute_includes command, "|| true"
    refute_includes command, "--skip"
    refute_includes command, "--ignored"
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

  def test_windows_print_coordinates_are_required_without_masking_emf_disagreements
    job = @workflow.fetch("jobs").fetch("windows-word-baselines")
    step = job.fetch("steps").find do |item|
      item["name"] == "Assert Windows Word print coordinates and preserve screen disagreements"
    end
    refute_nil step
    refute step.key?("if")
    refute step.fetch("continue-on-error", false)
    command = step.fetch("run")
    assert_includes command, "test_windows_print_coordinates.py"
    assert_includes command, "check_windows_print_coordinates.py --native-executable"
    refute_includes command, "|| true"
  end

  def test_lean_api_download_retains_the_pinned_checksum
    step = @workflow.fetch("jobs").fetch("crdt-worker-proof").fetch("steps").find do |item|
      item["name"] == "Install pinned Lean toolchain manager"
    end
    assert_equal "${{ github.token }}", step.fetch("env").fetch("GH_TOKEN")
    command = step.fetch("run")
    assert_includes command, "releases/assets/529114798"
    assert_includes command, "42b94d4244e8353142c456ec0e4ca6528fd898a6c604d4059f494e706e431f63  elan.tar.gz"
    assert_includes command, "sha256sum --check"
  end

  private

  def run_command(step_name)
    @steps.find { |step| step["name"] == step_name }&.fetch("run") ||
      flunk("missing #{step_name.inspect} step")
  end
end
