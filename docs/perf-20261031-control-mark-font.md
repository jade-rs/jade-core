# Control-mark font cache v1: hold decision

Candidate 82360c4e90b9bacbfe4f4410d06ab7d4a6f5b002. Baseline ba2d0905dc9d402525295db05d3ade250b4457db. Identical SDK 0.0.74 host. Preregistered 8+8 fixed interleaved runs, 32 unique Japanese preedits per run, 300 pages. Synthetic Chromium, not native latency. No retries or exclusions. Every run preserved complete saved source plus exactly one committed composition.

All-run values are medians of run-level medians / medians of run-level p95 values (not pooled p95).

| Metric | Baseline ms | Candidate ms |
|---|---:|---:|
| workerMs | 8.7525 / 25.9400 | 8.4725 / 24.7425 |
| bitmapMs | 9.4525 / 26.5425 | 9.1200 / 25.3675 |
| frameMs | 19.4425 / 29.2575 | 16.5075 / 35.7425 |

Frame p95 increased in both halves. In the second half, worker and bitmap p95 also increased. This fails the preregistered gate; do not promote or merge on these results. Host load recorded by the harness ranged from about 106 to 303 on an 8-core machine; unrelated rustc processes were observed immediately afterward. These observations cannot establish that this change caused the tail increase. Repeat only after checking for competing builds, retaining these original results.

Independent formatter-only native microdiagnostic, median ns/call across 8 alternating rounds (not an editor/native IME result). Its uncached branch uses literal prefix/family inputs while the cache formats runtime arguments, so compiler specialization can differ. Host contention also limits timing interpretation; use the separate allocation-count diagnostic to assess duplicate key allocations:

| Rotating styles | Uncached | Cached |
|---|---:|---:|
| 1 | 207.545 | 11.455 |
| 4 | 210.875 | 10.244 |
| 8 | 161.131 | 331.413 |

Cache misses allocate two unnecessary owned key strings and shift FIFO entries. An isolated prototype will remove those costs; it is not accepted without correctness and a new sealed matched comparison. Keep original observations and build artifacts unchanged.

## Follow-up representation

The next candidate stores prefix/family byte boundaries in the already owned descriptor, rather than allocating duplicate key strings. A circular FIFO slot replaces entries without shifting them. Empty prefixes keep the original replay formatter's two arguments. Exact float bits, full prefix/family text, the four-entry bound and held Rc validity remain unchanged.

Before this representation change, the allocation regression was strengthened to exercise the actual thread-local descriptor entry point and the formatter property test was expanded with UTF-8 prefixes. Those four tests pass against the first implementation.

An independent standalone counting-allocator diagnostic sampled 10,000 calls for each of 1, 4, 8 and 32 rotating keys, with empty and bold prefixes. After warmup, both caches allocate zero on hits. On every-miss sequences, the follow-up allocates/reallocates exactly as often as the matching uncached formatter; v1 performs additional key allocations. These are allocation counts, not editor or native timing results. The source/prototype/output hashes and all 24 observations are retained locally.

The follow-up remains draft until its correctness checks and a new matched, fixed-order browser comparison pass. Finish builds first and check for unrelated compiler processes before timing; retain rather than replace the original observations. No Word fixture, coordinate, tolerance or browser compatibility assertion is changed.

Follow-up editor all-feature library validation: 2,347 passed, 21 pre-existing ignored. This includes the strengthened UTF-8 formatter PBT and actual thread-local allocation-reuse regression. Clippy, frozen Word coordinates, release/browser validation and new matched timings remain pending for this follow-up.
