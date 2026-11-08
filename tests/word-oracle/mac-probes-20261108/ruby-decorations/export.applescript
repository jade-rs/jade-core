on run arguments
 tell application "Microsoft Word"
  open POSIX file (item 1 of arguments)
  set probeDoc to document "ruby-decorations.docx"
  save as probeDoc file name (item 2 of arguments) file format format PDF
  close probeDoc saving no
  return version
 end tell
end run
