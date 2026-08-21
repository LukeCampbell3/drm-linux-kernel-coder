# DRM Staged Deployment Results

Compiled C++23 staged session using the optimized speculative learning path, localized MDL promotion, deferred post-task consolidation, and the frozen semantic roots `OBSERVE`, `DERIVE`, `COMMIT`.

## Session summary

- Episodes: **708**
- Success: **708/708 (100%)**
- Semantic decisions: **831 (1.174/episode)**
- Permanent vocabulary: **22 words**
- Provisional cache at end: **20 fragments**
- Description-length reduction: **69.31%**
- Dense microcode task corpus: **1,046 bytes**
- Fused microcode image: **538 bytes**
- String/object structure: **4,407 bytes**
- Root vocabulary violations: **0**
- Recoveries: **4**, each followed by no repeat recovery
- Live Linux activity: **118 process spawns, 117 TCP requests, 173 Unix IPC requests, 59 timers, 1,225 commits**

## Stages

| Stage | Episodes | Semantic/task | New-task semantic | New tasks at 1 decision | Planner P95 | Consolidation P95 | Permanent words | Compression | Gate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| canary_core | 24 | 1.375 | 4.000 | 0.0% | 0.015 ms | 0.010 ms | 0 | 0.0% | PASS |
| single_user_alpha | 84 | 1.226 | 3.714 | 0.0% | 0.066 ms | 0.053 ms | 1 | 9.8% | PASS |
| connected_alpha | 72 | 1.833 | 2.250 | 37.5% | 0.185 ms | 2.060 ms | 14 | 46.5% | PASS |
| compositional_beta | 144 | 1.194 | 1.292 | 79.2% | 0.473 ms | 18.042 ms | 21 | 65.3% | PASS |
| resilience_beta | 60 | 1.117 | 1.042 | 95.8% | 0.531 ms | 24.157 ms | 22 | 68.7% | PASS |
| release_candidate | 324 | 1.000 | 1.000 | 100.0% | 0.191 ms | 0.001 ms | 22 | 69.3% | PASS |

## Key staged finding

The original inline permanent-MDL implementation failed the beta foreground-latency gate: compositional beta reached ~41 ms planner P95 and resilience beta ~66 ms. An exact localized MDL calculation reduced those costs, but still left ~17–23 ms P95. Moving permanent promotion to a post-task consolidation phase preserved **exactly the same semantic decisions and vocabulary state**, while lowering foreground P95 to ~0.47–0.53 ms in the same stages.

By the release-candidate stage, all 324 episodes required exactly **1 semantic decision**, all 12 new task identities also required **1 decision on first exposure**, permanent vocabulary stopped growing at 22 words, and consolidation P95 was effectively zero.

## Replication

Four GCC runs produced identical structural/learning metrics. Clang 17 reproduced the same deterministic outcome. A new nested PID/mount namespace could not be created in this host for this staged run because mounting `/proc` was denied; this is an environment restriction, not a task failure.
