-- Run with the absolute fixture path as the sole argument.
on run arguments
  set fixturePath to item 1 of arguments
  tell application "Microsoft Word"
    open POSIX file fixturePath
    set probeDoc to document "partial-style-numbering.docx"
    set resultLines to ""
    repeat with i from 1 to count paragraphs of probeDoc
      set lf to list format of text object of paragraph i of probeDoc
      set resultLines to resultLines & i & ": " & (list level number of lf) & " | " & (list string of lf) & linefeed
    end repeat
    close probeDoc saving no
    return resultLines
  end tell
end run
