# DRM Runtime Descent — Guarded Online Test

Frozen root vocabulary: `OBSERVE`, `DERIVE`, `COMMIT`.

## Runtime policy

1. If task structure is unstable, use the existing DRM developmental controller.
2. Once a task family is structurally stable for three consecutive executions, enter low-dimensional runtime descent.
3. Each family exposes at most three active execution coordinates.
4. Candidate `theta +/- step` configurations are tested on normal production episodes; no extra full task execution is required by the online optimizer.
5. Candidate output must remain valid; accepted moves require a material improvement threshold.
6. Step sizes shrink until adjacent-coordinate resolution is reached.
7. Before final certification, compare the candidate basin against the verified default baseline and roll back if the candidate is not materially faster.
8. During idle-time certification, test every adjacent coordinate with repeated paired measurements. A family is locally certified when no adjacent move is at least 3% or 0.05 ms faster with at least 4/5 paired wins.

## Frozen live result

- Base DRM regression: 99/99 success, 11 derived vocabulary entries, 4 recoveries, 4 repairs, 56.7766% description-length reduction.
- Runtime descent: 310/310 success.
- Families: 5/5 converged and 5/5 locally certified.
- Root vocabulary audit: pass (`OBSERVE`, `DERIVE`, `COMMIT` only).
- Extra full task executions during online optimization: 0.
- Online optimizer bookkeeping: 3.490252 ms total across 310 episodes.
- Candidate production episodes: 104.
- Baseline-anchor rollbacks: 1.
- Meaningfully faster families: 4/5; the fifth was rolled back to the default baseline.

Final paired validation:

| Family | Default ms | Final ms | Speedup | Break-even episodes |
|---|---:|---:|---:|---:|
| file | 202.486 | 135.505 | 1.494x | 6.29 |
| HTTP | 11.157 | 10.041 | 1.111x | 23.52 |
| proc | 1.535 | 1.535 | 1.000x | no adaptation |
| hash/process | 27.611 | 22.016 | 1.254x | 4.48 |
| timer | 4.601 | 3.872 | 1.188x | 24.04 |

The idle-time certificate is a test/maintenance cost and is kept separate from normal runtime optimizer cost.
