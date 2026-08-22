# server simulation summary

Episodes: 512

| engine | total wall (ms) | aggregate cpu (ms) | semantic tokens | commits | process spawns | tcp requests | ipc requests | failed episodes | permanent words | provisional words | verified specs | permanent specs | rolled back specs | reads avoided | transforms memoized |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASELINE_0_stateless | 630.081 | 100.000 | 1939 | 641 | 115 | 70 | 63 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_1_exact_cache | 607.987 | 100.000 | 1867 | 609 | 110 | 65 | 60 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_2_checkpoint_replay | 637.800 | 130.000 | 1867 | 641 | 115 | 70 | 63 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_3_static_macros | 170.523 | 40.000 | 1867 | 91 | 5 | 5 | 5 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_4_per_app_cache | 624.582 | 150.000 | 1867 | 609 | 110 | 65 | 60 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| DRM_A_permanent_only | 2627.441 | 2400.000 | 620 | 641 | 115 | 70 | 63 | 0 | 7 | 0 | 0 | 0 | 0 | 0 | 0 |
| DRM_B_provisional_permanent | 674.396 | 220.000 | 1141 | 641 | 115 | 70 | 63 | 0 | 16 | 17 | 0 | 0 | 0 | 0 | 0 |
| DRM_C_specialized | 675.582 | 230.000 | 1141 | 641 | 115 | 70 | 63 | 0 | 16 | 17 | 43 | 2 | 0 | 54 | 217 |

## Adversarial checks (spec S15)

- [PASS] warmup cost is included in the reported data -- cold (first-occurrence) rows are present in the metrics CSV
- [PASS] no engine reports a win by way of silently failed executions -- zero execution failures across every engine
- [PASS] DRM_B (no execution-skip mechanism) shows no fabricated wall-time win over BASELINE_0 -- DRM_B/BASELINE_0 wall-time ratio = 1.070 (expected roughly 0.5x-2.5x, since DRM_B cannot skip real work)
- [PASS] DRM_C never reports specialization use without the corresponding validated candidate on record -- reads_avoided=54, transforms_memoized=217, verified+permanent+rolled_back candidates=45
- [PASS] committed output is byte-identical across every engine (no engine silently changed observable behavior) -- 380 file comparisons across engines, all identical
- [PASS] every engine except the deliberately-naive BASELINE_3 produces every required durable output -- all 372 required outputs present for every engine except BASELINE_3 (which skipped 360/372 by design -- see its doc comment)
- [PASS] noise (single-occurrence, non-recurring patterns) is never promoted into learned vocabulary -- checked 20 noise patterns against every DRM engine's live vocabulary; none were learned
