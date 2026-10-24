# DOCX editing regression fixtures

`staging-base.docx` and `staging-review.docx` are synthetic Japanese contracts
reproduced from the fixture generators in jlsi/lawgue#19947. They contain no
customer documents. The review fixture adds another author's revisions,
comments, a header, and a footer.

`revision_workflows.rs` exercises these packages locally through the public
headless editor. It checks inherited fonts, replacement underlining, paragraph
insertion, targeted rejection, table integrity, and preservation of unrelated
OOXML parts and revision metadata. Set `DOCX_VERIFICATION_DIR` to export the
results for inspection in LibreOffice. The T00–T04 output names identify local
fixture variants; these tests do not call staging or Copilot.

The same suite has independent string/grid state machines. Each generated
operation edits and reloads a DOCX package before the next operation: 128 text
workflows of up to 40 operations and 64 table workflows of up to 30 operations.
