# Mac and Windows Word coordinate expectations

`platform-coordinates.json` registers identical DOCX inputs with independent
Word for Mac and Word for Windows references. An OS difference in Word output
is preserved in those references; it is not a renderer regression by itself.

For every registered input:

- `source` identifies the original Mac DOCX by SHA-256. Windows captures must
  contain those exact source bytes.
- `mac_manifest` identifies the original Mac capture. Its `pages.json.gz`
  contains Mac expectations and is checked against the manifest's SHA-256.
- `windows` retains separate captures with the Word product version, capture
  identity and original output hashes. Archived captures additionally verify
  the Word executable hash. A later capture must not replace an older one.

GitHub Actions asserts each platform against its own reference:

- `Mac Word browser coordinate match` runs on macOS and renders the imported
  Mac sources against Mac Word coordinates. Every shared registry case must
  be included. `Mac Word coordinate match` separately runs native Mac tests.
- `Windows Word baselines by version` runs on Windows and invokes
  `check_platform_coordinates.py --native-executable ...` against the Windows
  captures, including original EMF, Word PDF and native PDF coordinates.
- Both native OS jobs run `test_platform_coordinates.py` and the reference
  integrity verifier. The tests include a real source whose Mac and Windows
  coordinates differ: each original passes its own comparison, while swapping
  the OS expectations fails. All shared pairs also reject X/Y displacement.

These checks fail on coordinate drift, missing text, page changes, modified
originals or removed registered captures. They do not select whichever OS
reference happens to pass. Original Word tolerances remain unchanged.

Some imported Mac captures predate version recording. Their missing version
is retained as missing provenance; it must not be filled using a Windows
version or a guessed Mac build. New reference data must come from the actual
Word application, rather than from the renderer under test.
