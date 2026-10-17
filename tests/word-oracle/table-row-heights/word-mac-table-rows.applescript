-- Word for Mac oracle for table row positions.
--
-- Usage:
--   osascript word-mac-table-rows.applescript <path-to.docx> <table index>
--
-- Prints, for every row of the table, the vertical position (points from the
-- page top) of the first cell's text, the persisted row height / rule, and the
-- position of the paragraph that follows the table. Consecutive positions are
-- Word's painted row heights. Word must be able to open the file directly
-- (e.g. from ~/Downloads); once Word sits on its document gallery with no
-- document open, `open` silently fails, so quit Word between runs.
on run argv
  set thePath to item 1 of argv
  set tblIndex to (item 2 of argv) as integer
  tell application "Microsoft Word"
    if (count of documents) = 0 then
      open (POSIX file thePath as string)
    end if
    set tries to 0
    repeat until (count of documents) > 0 or tries > 30
      delay 0.5
      set tries to tries + 1
    end repeat
    set d to document 1
    set out to ""
    set t to table tblIndex of d
    set n to count of rows of t
    repeat with i from 1 to n
      set r to row i of t
      set c to cell 1 of r
      set y to (get range information (text object of c) information type vertical position relative to page)
      set out to out & "row " & i & " y=" & y & " height=" & (height of r) & " rule=" & (height rule of r) & linefeed
    end repeat
    set tr to text object of t
    set e to end of content of tr
    set r2 to create range d start e end e
    set ya to (get range information r2 information type vertical position relative to page)
    set out to out & "after y=" & ya & " page=" & (get range information r2 information type active end page number) & linefeed
    return out
  end tell
end run
