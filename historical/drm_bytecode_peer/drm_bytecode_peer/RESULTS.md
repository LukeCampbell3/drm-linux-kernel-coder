# Frozen Results

Compiler baseline: GCC 14.2.0, C++20, `-O3 -Wall -Wextra -Wpedantic -Werror`.

Developmental regression over the same 99-episode workload:

- semantic decisions: 214
- derived vocabulary: 11
- historical recoveries: 4
- local repairs: 4
- root vocabulary: exactly `OBSERVE`, `DERIVE`, `COMMIT`
- all 57 final task programs round-trip exactly through both bytecode formats
- 12/12 selected live Linux workflow shapes completed successfully under graph, explicit bytecode, fused explicit bytecode, dense micro-op, and dense fused-block execution
- historical old/new blocks remained simultaneously addressable and immutable

Hot representation for 57 final tasks:

| Format | Bytes | Reduction vs string graph |
|---|---:|---:|
| String/capability graph | 3,807 | baseline |
| Explicit 16-bit O/D/C bytecode | 892 | 76.57% |
| Explicit fused blocks + 16-bit task refs | 334 | 91.23% |
| Dense 8-bit micro-ops | 273 | 92.83% |
| Dense fused blocks + 16-bit task refs | 182 | 95.22% |

Five repeated GCC dispatch measurements (per task program):

- string graph: 122.45 ns mean
- explicit O/D/C: 26.22 ns mean (~4.67x faster)
- explicit fused block: 23.25 ns mean (~5.27x)
- dense micro-op: 16.49 ns mean (~7.81x)
- dense fused block: 13.19 ns mean (~9.30x)

Relative to explicit O/D/C bytecode, the dense short form used ~69.4% fewer bytes and ~37% less dispatch time in this GCC run; fused dense blocks used ~45.5% fewer bytes and ~43% less dispatch time than fused explicit blocks.

Clang 17 independently passed the same self-tests and reproduced all deterministic structural metrics. Absolute microbenchmark ratios differed by compiler, but the dense formats remained faster than the string graph and explicit O/D/C forms.

A full run measured about 4.7 MB maximum RSS in this host. Real workflow wall times are dominated by the underlying filesystem/process/socket/timer operations and are included only as correctness checks, not as evidence of bytecode speedup.
