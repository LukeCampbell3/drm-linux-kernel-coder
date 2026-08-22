# desktop simulation summary

Episodes: 303

| engine | total wall (ms) | aggregate cpu (ms) | semantic tokens | commits | process spawns | tcp requests | ipc requests | failed episodes | permanent words | provisional words | verified specs | permanent specs | rolled back specs | reads avoided | transforms memoized |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| BASELINE_0_stateless | 257.980 | 50.000 | 1110 | 378 | 39 | 37 | 32 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_1_exact_cache | 243.597 | 50.000 | 1058 | 354 | 35 | 33 | 30 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_2_checkpoint_replay | 263.397 | 60.000 | 1058 | 378 | 39 | 37 | 32 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_3_static_macros | 104.530 | 20.000 | 1058 | 94 | 2 | 5 | 2 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| BASELINE_4_per_app_cache | 318.425 | 70.000 | 1058 | 354 | 35 | 33 | 30 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| DRM_A_permanent_only | 900.448 | 760.000 | 405 | 378 | 39 | 37 | 32 | 0 | 8 | 0 | 0 | 0 | 0 | 0 | 0 |
| DRM_B_provisional_permanent | 288.460 | 90.000 | 721 | 378 | 39 | 37 | 32 | 0 | 6 | 7 | 0 | 0 | 0 | 0 | 0 |
| DRM_C_specialized | 276.198 | 90.000 | 721 | 378 | 39 | 37 | 32 | 0 | 6 | 7 | 25 | 0 | 0 | 16 | 83 |

## Adversarial checks (spec S15)

- [PASS] warmup cost is included in the reported data -- cold (first-occurrence) rows are present in the metrics CSV
- [PASS] no engine reports a win by way of silently failed executions -- zero execution failures across every engine
- [PASS] DRM_B (no execution-skip mechanism) shows no fabricated wall-time win over BASELINE_0 -- DRM_B/BASELINE_0 wall-time ratio = 1.118 (expected roughly 0.5x-2.5x, since DRM_B cannot skip real work)
- [PASS] DRM_C never reports specialization use without the corresponding validated candidate on record -- reads_avoided=16, transforms_memoized=83, verified+permanent+rolled_back candidates=25
- [PASS] committed output is byte-identical across every engine (no engine silently changed observable behavior) -- 191 file comparisons across engines, all identical
- [PASS] every engine except the deliberately-naive BASELINE_3 produces every required durable output -- all 185 required outputs present for every engine except BASELINE_3 (which skipped 174/185 by design -- see its doc comment)
- [PASS] noise (single-occurrence, non-recurring patterns) is never promoted into learned vocabulary -- checked 20 noise patterns against every DRM engine's live vocabulary; none were learned
