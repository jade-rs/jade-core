# Windows table style controls

Nine captures preserve 77 Word 16.0.19127.20936 observations, including one
unchanged older public input. The generator reproduces the other 76 DOCX
files byte for byte from two pinned source inputs and validates every XML
part. Original ZIPs retain the exact captured metadata, inputs, PDF and EMF
bytes. Direct accepted copies normalize only metadata JSON line endings;
their hashes and derived EMF references are pinned independently.

The complete observation registry contains 254 captures / 1,023 documents.
All 77 controls are also accepted four-plane coordinate regressions: raw EMF,
unclipped Word PDF text, visible Word PDF text, and independently painted
native PDF text, using the established 0.375pt absolute X/Y limit. Only font
programs from the original Word PDFs enter the renderer. Reference origins
are never renderer inputs. The original Word PDFs have no vector ink in 70
controls; CI additionally rejects visible native border or editing-guide
paint in those cases, including guides represented by filled rectangles.

The controls isolate these source distinctions:

- Parent/child `basedOn` indentation, direct-format overrides, and compatibility
  modes 12/14/15. A child nil/auto/percentage declaration resets its parent's
  DXA indentation without becoming a zero DXA declaration.
- Built-in Normal Table versus a custom default. On this Windows Word build,
  the built-in style uses zero indentation even when it declares a positive
  DXA, nil, auto or percentage value. A custom default retains its authored
  positive indentation.
- Primary `styles.xml` versus `stylesWithEffects.xml`. The historical
  `no-styles` filenames remove only the primary part; follow-up controls
  remove both parts or vary each independently. The effects copy can retain
  Normal Table when the primary lacks it, but an effects-only custom table
  style must not replace the primary style set. A primary custom default
  takes precedence. Packages with neither default remain distinct.
- Package/root/section/row variations, cell margins, authored positive/nil
  borders, and a preceding paragraph. XML-backed tables without declared
  borders must not acquire a synthetic 0.75pt border or reserve its height.

Source-driven indentation/default handling and clean PDF guide suppression
are Windows-specific. The Mac style-load and PDF paint paths, committed Mac
originals, and coordinate tolerances remain unchanged. Mac regression CI
continues to compare against its own original Word coordinates.
