import Std

set_option autoImplicit false
set_option warningAsError true

/-!
A specification of worker/capture.ts, not a proof of the TypeScript runtime or Loro.
`acknowledged`/`captured` count accepted edit batches. The seeded trace test checks
observable implementation behavior against these transitions, including stale jobs.
Timers and yields only schedule work; they never commit a native document.
-/
namespace Jade.Capture

structure State where
  generation : Nat := 0
  acknowledged : Nat := 0
  captured : Nat := 0
  dirty : Bool := false
  failed : Bool := false
  deriving DecidableEq, Repr

def Valid (s : State) : Prop :=
  s.captured ≤ s.acknowledged ∧
  (s.dirty = false → s.captured = s.acknowledged) ∧
  (s.failed = true → s.dirty = true)

def stage (s : State) : State :=
  if s.failed then s else
    { s with generation := s.generation + 1,
             acknowledged := s.acknowledged + 1, dirty := true }

def flush (s : State) (ok : Bool) : State :=
  let next := { s with generation := s.generation + 1 }
  if s.failed || !s.dirty then next
  else if ok then { next with captured := s.acknowledged, dirty := false }
  else { next with failed := true }

def run (s : State) (expected : Nat) (ok : Bool) : State :=
  if s.dirty && decide (expected = s.generation) then flush s ok else s

def yieldJob (s : State) (expected : Nat) : State :=
  if s.dirty && decide (expected = s.generation) then
    { s with generation := s.generation + 1 } else s

inductive Event where
  | edit
  | barrier (ok : Bool)
  | job (expected : Nat) (ok : Bool)
  | yieldJob (expected : Nat)
  | readOrRejectedEdit

def step (s : State) : Event → State
  | .edit => stage s
  | .barrier ok => flush s ok
  | .job expected ok => run s expected ok
  | .yieldJob expected => yieldJob s expected
  | .readOrRejectedEdit => s

theorem initial_valid : Valid {} := by simp [Valid]

theorem stage_valid (s : State) (h : Valid s) : Valid (stage s) := by
  unfold stage
  split
  · exact h
  · rcases h with ⟨hle, _, _⟩
    simp_all [Valid]
    omega

theorem flush_valid (s : State) (ok : Bool) (h : Valid s) : Valid (flush s ok) := by
  rcases h with ⟨hle, hclean, hfailed⟩
  cases hf : s.failed <;> cases hd : s.dirty <;> cases ok <;>
    simp_all [flush, Valid]

theorem step_valid (s : State) (e : Event) (h : Valid s) : Valid (step s e) := by
  cases e with
  | edit => exact stage_valid s h
  | barrier ok => exact flush_valid s ok h
  | job expected ok =>
    simp only [step, run]
    split
    · exact flush_valid s ok h
    · exact h
  | yieldJob expected =>
    simp only [step, yieldJob]
    split <;> exact h
  | readOrRejectedEdit => exact h

theorem trace_valid (events : List Event) : Valid (events.foldl step {}) := by
  have go : ∀ (xs : List Event) (s : State), Valid s → Valid (xs.foldl step s) := by
    intro xs
    induction xs with
    | nil => intro s h; exact h
    | cons e xs ih => intro s h; exact ih (step s e) (step_valid s e h)
  exact go events {} initial_valid

-- An obsolete queued job cannot capture or modify a newer edit.
theorem stale_job (s : State) (expected : Nat) (ok : Bool)
    (h : expected ≠ s.generation) : run s expected ok = s := by
  simp [run, h]

-- An acknowledged edit remains native-only until an explicit capture.
theorem stage_keeps_captured (s : State) : (stage s).captured = s.captured := by
  unfold stage
  split <;> rfl

-- A successful save barrier includes every acknowledged edit, even when clean.
theorem save_caught_up (s : State) (h : Valid s) (hf : s.failed = false) :
    (flush s true).dirty = false ∧ (flush s true).captured = s.acknowledged := by
  rcases h with ⟨_, hclean, _⟩
  cases hd : s.dirty <;> simp_all [flush]

-- Capture failure retains the acknowledged edits and makes dependent edits inert.
theorem failed_capture_retains_input (s : State)
    (hf : s.failed = false) (hd : s.dirty = true) :
    (flush s false).acknowledged = s.acknowledged ∧
    (flush s false).captured = s.captured ∧
    (flush s false).dirty = true ∧ (flush s false).failed = true := by
  simp [flush, hf, hd]

theorem failed_stage_blocked (s : State) (hf : s.failed = true) : stage s = s := by
  simp [stage, hf]

-- Yielding to interaction never captures input, and invalidates the old job.
theorem yield_keeps_captured (s : State) (expected : Nat) :
    (yieldJob s expected).captured = s.captured := by
  unfold yieldJob
  split <;> rfl

theorem yielded_job_obsolete (s : State) (ok : Bool) (hd : s.dirty = true) :
    run (yieldJob s s.generation) s.generation ok = yieldJob s s.generation := by
  apply stale_job
  simp [yieldJob, hd]

end Jade.Capture

/-! Executable oracle: the PBT invokes the very `step` proved above, rather than
maintaining an unverified translation as its only reference. Each line is an
operation; each response is generation/acknowledged/captured/dirty/failed. -/
def main : IO Unit := do
  let input ← IO.getStdin
  let output ← IO.getStdout
  let mut state : Jade.Capture.State := {}
  repeat
    let line ← input.getLine
    if line.isEmpty then break
    let parts := line.trim.splitOn " "
    let event ← match parts with
      | ["reset"] => pure none
      | ["edit"] => pure (some Jade.Capture.Event.edit)
      | ["barrier", "1"] => pure (some (.barrier true))
      | ["barrier", "0"] => pure (some (.barrier false))
      | ["job", expected, ok] =>
        match expected.toNat?, ok with
        | some n, "1" => pure (some (.job n true))
        | some n, "0" => pure (some (.job n false))
        | _, _ => throw (IO.userError "invalid job")
      | ["yield", expected] =>
        match expected.toNat? with
        | some n => pure (some (.yieldJob n))
        | none => throw (IO.userError "invalid generation")
      | ["read"] => pure (some .readOrRejectedEdit)
      | _ => throw (IO.userError s!"invalid command: {line}")
    state := match event with
      | none => {}
      | some event => Jade.Capture.step state event
    let dirty := if state.dirty then 1 else 0
    let failed := if state.failed then 1 else 0
    output.putStrLn s!"{state.generation} {state.acknowledged} {state.captured} {dirty} {failed}"
