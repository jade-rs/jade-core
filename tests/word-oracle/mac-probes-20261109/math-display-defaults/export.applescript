on run arguments
 if (count of arguments) is not 2 then error "Expected absolute probe DOCX and PDF paths"
 set sourcePath to item 1 of arguments
 set outputPath to item 2 of arguments
 set probeName to name of (info for (POSIX file sourcePath))
 tell application "Microsoft Word"
  open POSIX file sourcePath
  set probeDoc to document probeName
  save as probeDoc file name outputPath file format format PDF
  close probeDoc saving no
  return version
 end tell
end run
