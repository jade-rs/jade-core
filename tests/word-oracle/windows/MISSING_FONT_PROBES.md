Windows Word missing-font controls
==================================

The unchanged `public/fixtures/hello.docx` requests Hiragino Kaku Gothic ProN W3
for Latin body text, with `w:family="roman"` and no PANOSE classification. On
this Windows workstation that face is absent. Word uses Cambria; using the old
Times New Roman fallback moved the baseline by approximately 1.65 pt and changed
the horizontal advances. The fix resolves the available family before selecting
Word line metrics, and uses the same family for measurement and painting.

`generate_windows_missing_font_probes.py` reproduces nine additional DOCX inputs
byte for byte from the pinned original. Body text, breaks and section geometry
are unchanged. The controls vary the absent family name and font-table design
class. Each accepted capture contains one document opened read-only in a fresh
Word application. Distinct invented Latin and Japanese names confirm that this
is not a special case for the Hiragino spelling:

| No-PANOSE family class | Face used by the captured Windows Word |
| --- | --- |
| roman | Cambria |
| swiss | Calibri |
| auto | Calibri |
| modern, including fixed pitch | Calibri |

The runtime change applies to Windows Latin slots. An installed requested face
still wins. Authored alternative names, known aliases and registered compatible
stand-ins retain precedence. PANOSE-qualified fonts, unmeasured classes,
East Asian and complex-script slots retain their existing policy. Mac retains
its existing font stacks and line metrics.

All ten accepted cases use Windows Word **16.0.19127.20936**, executable SHA-256
`7cdd5aac841376d950db492b3a8f026d422844dc7a07787b6d0240f606501312`.
The original's capture ID is `20261114T210924740Z-80d17c55`; the nine independent
captures are `20261114T220509174Z-2683902f`, `20261114T220512990Z-73ec2dd1`,
`20261114T220516290Z-0f07ef16`, `20261114T220519401Z-b217a427`,
`20261114T220522462Z-de8edfdd`, `20261114T220525529Z-bfc4e60e`,
`20261114T220527187Z-ca3d087e`, `20261114T220530227Z-fb8039c2` and
`20261114T220533305Z-0d6cb896`. Version directories contain original inputs,
PDFs, per-page EMFs, source hashes and capture metadata. Immutable observation
ZIPs preserve the original metadata bytes. Older Word versions are retained.

The earlier nine-document capture `20261114T215723605Z-c3435584` is preserved
as an **unpromoted observation**. It reused one invented missing font name
inside one Word process: after the first substitution, later design classes
kept that process's earlier selected face. It must not replace the isolated
captures. When collecting new missing-face controls, invoke the capture script
separately for each source so each invocation creates a fresh Word application.

The original uses `GM_ADVANCED` EMF text records with a pure world translation.
The extractor follows the recorded translation and the whole-page device canvas;
the unused per-record scale values are ignored in this graphics mode. Scaling,
rotation, unknown units and unsupported transforms still fail. The independent
Word PDF confirms the derived EMF positions. No fitting or native-output positions
are used to construct Word expectations. Microsoft's relevant specifications:
[EMR_EXTTEXTOUTW](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-emf/a59a79ac-328e-492d-a34d-e02727af6edf)
and [ModifyWorldTransform](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-emf/e6bb2996-195f-473f-80c6-9dc1afe474f9).

CI re-extracts the original evidence, verifies the byte-identical generated
inputs, and asserts all four coordinate planes with the unchanged 0.375 pt limit
on both axes. The renderer receives only DOCX bytes and font programs extracted
from the original PDF; expected coordinates are not renderer inputs. The Mac
native and browser coordinate jobs remain mandatory on macOS with their original
Word references and tolerances.
