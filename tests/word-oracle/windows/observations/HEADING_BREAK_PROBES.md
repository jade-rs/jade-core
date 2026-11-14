# Windows paragraph spacing after manual page breaks

Three original Word 16.0.19127.20936 captures retain 52 immutable DOCX/PDF/EMF
observations, complete executable identity and capture provenance.

The accepted capture is `20261114T185441333Z-8f9ddd27`, containing 26
`ppr-only` inputs. Character spacing carries only its `w:val="0"` attribute;
paragraph before spacing appears only in paragraph properties. Generator tests
check this distinction explicitly. Sixteen controls vary Heading 1 before
spacing (zero/480 twips), compatibility modes 12/14/15, both paragraph-final
manual break forms, document start and `pageBreakBefore`. Ten holdouts vary
Normal style and direct/inherited `pageBreakBefore` after the manual breaks.
These compatibility modes are document settings; every measurement uses the
Word product version above, while older registered versions remain intact.

Captures `20261114T174845835Z-eef8d731` and `20261114T181208382Z-8f7d9f6b`
preserve 26 earlier observations whose generator also placed a paragraph-only
before attribute in character-spacing elements. Their exact inputs and
original output remain available as observations. Accepted gates use only the
freshly captured paragraph-only controls, with separate source hashes.

Twelve accepted controls exposed a vertical error: the next paragraph retained
24pt of before spacing after a paragraph-final manual page break, while Word
omits that gap. Fourteen controls require preserved spacing behavior, including
document start and direct/inherited `pageBreakBefore`. The Windows fix uses
source break semantics and preserves the existing Mac spacing policy.

All 26 accepted controls pass raw EMF, unclipped Word PDF, visible Word PDF and
separately painted native PDF origins at the existing absolute 0.375pt X/Y
limit. They add 640 origins: the registry asserts 329 cases / 103,875 origins.
All 26 original Word PDFs have no vector ink, so extra native vector paint is
also rejected. Font programs and DOCX bytes are the only oracle-derived inputs
to the renderer; expected origins never enter it.

The complete observation registry contains 257 captures / 1,075 documents /
1,048 source byte groups. ZIPs preserve original captured metadata bytes;
direct copies use independently represented LF metadata and reconstructed EMF
references. `generate_windows_heading_break_probes.py` reproduces all 52 source
inputs from the pinned header/footer source, including the legacy observations.
Mac originals and tolerances remain unchanged; macOS native/browser coordinate
assertions continue to run unconditionally in CI.
