import Std
set_option autoImplicit false
set_option warningAsError true

/-! Persistence policy model; this does not prove the JS queue or transport.
The PBT compares the actual session's clean markers with this executable oracle. -/
namespace Jade.Session

def marker (before after clean : Nat) : Nat :=
  if before = clean ∧ after = before + 1 then after else 0

def cursor (old next : Nat) (accepted : Bool) : Nat := if accepted then next else old

theorem advance_only_lone_import (before after clean : Nat) (h : marker before after clean ≠ 0) :
    before = clean ∧ after = before + 1 := by
  unfold marker at h
  split at h
  · assumption
  · contradiction

theorem typing_before_stays_dirty (before after clean : Nat) (h : before ≠ clean) :
    marker before after clean = 0 := by simp [marker, h]

theorem typing_during_stays_dirty (before after clean : Nat) (h : after ≠ before + 1) :
    marker before after clean = 0 := by simp [marker, h]

theorem failed_import_retries_cursor (old next : Nat) : cursor old next false = old := by rfl

theorem accepted_import_advances_cursor (old next : Nat) : cursor old next true = next := by rfl
end Jade.Session

def main : IO Unit := do
  let input ← IO.getStdin
  let output ← IO.getStdout
  repeat
    let line ← input.getLine
    if line.isEmpty then break
    match (line.trim.splitOn " ").map String.toNat? with
    | [some before, some after, some clean, some old, some next, some ok] =>
      output.putStrLn s!"{Jade.Session.marker before after clean} {Jade.Session.cursor old next (ok == 1)}"
    | _ => throw (IO.userError "invalid session case")
