These 32 controls retain actual Windows Word 16.0.19127.20936 output for
justified paragraphs with zero, one or three ASCII spaces before a manual
break or paragraph ending. Mixed Japanese/Latin controls vary the East Asia
font and compatibility modes 14/15; eight Latin-only controls isolate spaces.
Two mixed controls reproduce their previously captured parent DOCX bytes.

`hard-break-space-observations.json` binds each source, original capture and
promotion to immutable SHA-256 values. Versioned directories retain original
DOCX/PDF/EMF/XPS files and the full capture bundles, including Word executable
identity, actual embedded font versions/program hashes and capture tools.
`generate_windows_hard_break_space_probes.py` reproduces every source byte.

CI requires 26 controls against original Word screen EMF, unclipped PDF,
visible PDF and native painted PDF, including paragraph background geometry
and color. Twenty also require original XPS origins on both native and live
render paths. Negative tests displace both axes in each path and reject
missing, swapped or silently added promotions.

Six mixed-script compatibility-mode-14 manual-break controls still have
coordinate differences. Twelve mixed-script manual-break XPS comparisons
remain unpromoted. Their original outputs remain available for diagnosis;
these records do not claim coordinate agreement. Existing tolerances and
Mac originals are unchanged. Mac and Windows CI use their own platform
references; changing Windows expectations cannot replace Mac expectations.
