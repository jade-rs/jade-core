# Reopening fonts on Windows

Repeated native editing histories reopened the same Noto font bytes under five
family aliases. Every registration created a new DirectWrite typeface. A local
24-case formatting run passed alone but grew to 3.48 GB of private memory; the
larger serial suite subsequently aborted on a font-sized allocation.

On Windows only, registrations now reuse a typeface when the font bytes and
collection index are exactly equal. Hashes narrow the search; full byte equality
prevents collisions from selecting a different font. Family, weight and italic
aliases remain separate registrations. The per-thread cache retains at most
64 MiB of input bytes and 64 faces, evicting the least recently used entry.
Oversized fonts still load without entering the cache. An evicted typeface stays
valid while a painter retains its reference.

With reuse, another run of the same 24-case formatting property completed with
sampled private memory below 273 MB. These observations are diagnostic rather
than a timing benchmark; property sequences vary between runs. Reusing a face
does not establish a process-wide memory bound for arbitrary distinct fonts.

The Mac registration path is unchanged under conditional compilation. No Word
reference, origin adjustment or comparison tolerance changes. Word captures
remain separated by full product version and immutable capture ID.

Windows CI checks content reuse across reopened painters, different fonts and
invalid collection indices, existing style and OpenType rendering, every
registered Word coordinate baseline, and the native document, table and editing
history suites. The independent Mac coordinate job and full Mac native suite
remain required.
