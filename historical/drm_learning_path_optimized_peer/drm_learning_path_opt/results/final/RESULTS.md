# DRM Fast Vocabulary Learning — Frozen Result

The optimized learner separates speculative transfer vocabulary from permanent MDL vocabulary. Speculative fragments may be used immediately for semantic compression, but only the original conservative MDL controller can change permanent vocabulary.

## Frozen result

- Root vocabulary remains `OBSERVE / DERIVE / COMMIT`.
- Semantic decisions: **214 -> 166** (22.4% reduction).
- First useful abstraction: **episode 22 -> episode 2**.
- Permanent vocabulary: **11 -> 11** words; no permanent overgrowth.
- Permanent final commit remains episode **85**; structural certainty is deliberately conservative.
- First-seen task-family execution becomes permanently one semantic decision from episode **44**, versus **58** originally.
- Speculative cache: **20 fragments**, **55 raw dense micro-op bytes**; about **95 B** as micro-op payload plus 16-bit block references, excluding counters/allocator metadata.
- Ten-run mean learning time: baseline **141.737 ms**, optimized **147.359 ms**. Added planner work: **56.8 us/episode**.
- GCC and Clang reproduce the deterministic 214 -> 166 semantic result.

## Novel family convergence

| 8-task block | Baseline decisions/task | Optimized decisions/task |
|---:|---:|---:|
| 1 | 4.375 | 3.625 |
| 2 | 4.125 | 3.000 |
| 3 | 4.000 | 1.625 |
| 4 | 1.625 | 1.000 |
| 5 | 1.375 | 1.000 |

## Admission policy

1. Observe repeated raw capability subsequences.
2. Admit a provisional motif only with cross-task evidence and non-negative local description payoff.
3. Use a small dynamic-programming tokenizer over permanent + provisional motifs; this guarantees speculation cannot increase semantic token count.
4. Keep permanent growth under the existing MDL rule.
5. Remove provisional motifs once an equivalent permanent word exists, or expire them after a no-transfer grace interval.
6. Cap the speculative cache at 20 for this workload; the sweep plateaus at 20-24, so larger caches do not improve the result.

An attempted aggressive permanent-promotion policy was rejected: it reduced decisions but expanded permanent vocabulary to 32+ words. An attempted churn-based eviction policy was also rejected because it worsened semantic cost from 166 to 173.
