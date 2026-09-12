# Bash-rule latency profile and baseline

## Finding

The initial profile was dominated by **rebuilding the rule engine for each CLI invocation**, not evaluating the command
or rendering `--explain`. Before optimization, on the committed representative policy, `BashRuleEngine::from_config`
plus destruction took **41.56 ms**. A warm `evaluate_sync("ls", ..., Diagnostics)` took **2.08 µs**, and pretty-JSON
serialization of its trace took **1.81 µs**.

The original baseline is retained below; [Shared fragment matcher](#shared-fragment-matcher) and
[NFA-only validation](#nfa-only-validation) record the subsequent optimizations.

## Original-process profiling

The original 53 ms measurement did not include a reproducible command or config path. With permission, profiling used
the installed policy and representative `ls`, `ls && git status`, and `ls >/dev/null 2>&1` inputs. These strings were
analyzed as policy input, never executed as shell commands.

- Installed policy: 154,413 bytes, 193 Bash rules, 165 tool rules, 106 user fragments.
- Original policy SHA-256: `4128e2cc4e2d764b3be1829f743b9084e14edbd21153fdf3d32fe2bbb394d508`.
- Installed executable: `/nix/store/l41ifmibc9p8zkhc5dnmcnsy70gjh3bc-moriarty-0.1.0/bin/moriarty`, statically linked
  musl, with symbols. Its source revision was not established; the function baseline below uses this repository instead.
- Initial wall-clock measurements used Python `subprocess.run`, captured stdout/stderr, five warm-up processes, then 30
  measured processes per case. `--json ls` had a 55.16 ms median (50.20–67.79 ms min–max); `--json --explain ls` had a
  64.33 ms median (50.48–92.60 ms). Later runs had severe scheduling outliers, so these are reproduction evidence, not a
  controlled cross-command comparison. This approximately reproduces the reported latency, not its exact input.
- `perf record -e cpu-clock:u -F 997 --call-graph dwarf,16384` sampled 100 sequential installed-binary invocations of
  `test bash-rules -c <installed-policy> --json --explain ls`: 3,377 samples, no lost samples. The loop driver was
  included in the sample denominator. Regex automaton construction dominated recovered stacks: Thompson NFA
  construction, DFA determinization, regex syntax parsing, and HIR translation. Example self-sample costs were
  `__libc_free` 11.07%, `memcpy` 8.82%, `__libc_malloc_impl` 6.78%, `__unlock` 6.34%, and `__lock` 5.21%.
  `regex_automata::meta::strategy::new` had 31.09% inclusive samples. Inclusive percentages overlap and must not be
  added; incomplete DWARF unwinding prevents exact whole-phase attribution from those stacks.
- `strace -f -c` observed 33 thread creations, 3,383 `mmap` calls, and 3,354 `munmap` calls in one invocation. File
  reads consumed only 116 µs of traced syscall time. Futex/epoll totals include simultaneous idle-thread waits and
  tracing overhead; they are **not** evidence that the application spends seconds blocked.

`test_runner::test_bash_rules` loads TOML and calls `BashRuleEngine::from_config` every time. At the profiled revision,
`compile_with_diagnostics` expanded each pattern, constructed an individual `Regex`, and constructed separate
command/redirect `RegexSet`s. `expand_fragments` additionally compiled the same fragment-marker regex on **every** call,
including patterns with no fragment references. Matching is a later, much smaller operation.

## Self-contained function benchmarks

Run from the repository, without user config, installed executables, or environment-variable setup:

```bash
cargo bench -p moriarty --bench bash_rules
```

Each Criterion case calls functions directly. None spawns the Moriarty executable. The harness reuses the binary's
private Rust modules; it does not load or invoke an installed binary. `cargo bench` uses an optimized profile inheriting
`release`: opt-level 3, fat LTO, one codegen unit, and debug symbols retained. Regular tests still run with Nextest.

The committed fixture is `crates/moriarty/benches/fixtures/bash_rules.toml`: a sanitized snapshot preserving rule order,
regex/fragment structure, action kinds, mode/locality conditions, and counts. Names, messages, personal/project
identifiers, and comments were replaced or removed. It is **not** a recommended security policy. It has 186 command
rules and seven redirect rules; its 165 tool rules are parsed but not compiled by this Bash-only command.

- Fixture size: 68,224 bytes.
- Fixture SHA-256: `c5cc97fdadd1f4a952557c144b16962d4f8379684d89a25ef38f77807c5b1ec5`.
- Sanitization means this is a representative compilation workload, **not** an identical TOML-parsing workload to the
  original 154 KB file. The two sets of times must not be treated as additive parts of the exact original 53 ms.
- Shared setup validation asserts the rule/fragment counts, rejects any dropped compilation rule, and checks that
  representative commands still evaluate to Allow. A Nextest test exercises the same checks without Criterion; `--test`
  runs every benchmark once as a smoke check.

### Saved baseline

Baseline name: `pre-optimization-musl`. Recorded 2026-09-12 UTC against benchmark commit `c64ce5e6`, after the generated
Hakari update. Linux deployment uses musl, so the primary baseline deliberately uses that target rather than silently
substituting the host's glibc allocator. This baseline uses musl's **system allocator** and predates the mimalloc
deployment selection.

```bash
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  --sample-size 50 --warm-up-time 1 --measurement-time 3 \
  --save-baseline pre-optimization-musl

# After an approved optimization, compare without replacing the baseline:
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  --sample-size 50 --warm-up-time 1 --measurement-time 3 \
  --baseline pre-optimization-musl

# Functional smoke check, not a measurement run:
cargo bench -p moriarty --bench bash_rules -- --test
```

The musl target must be installed; the project's Nix development shell provides it on Linux. Native `cargo bench`
remains supported, but native and musl results need separate baseline names. Use the same target, machine, fixture, and
parameters for a meaningful comparison.

Machine: Intel Core i9-13900K, 32 logical CPUs; Linux 6.18.45-xanmod1; Rust 1.96.0 (`ac68faa20`); Criterion 0.8.2.
`TOKIO_WORKER_THREADS`, `RUST_LOG`, and custom Rust flags were unset. Cargo's benchmark cwd was `crates/moriarty`.
Measurements were not CPU-pinned, governor-controlled, or isolated from other applications. A preceding run measured
39.29 ms for engine construction/destruction and 2.19 ms for runtime construction/destruction, versus 41.56 ms and 0.82
ms in the final run. Treat small changes cautiously, particularly thread-startup timings; no speedup claim is based on
that repeat. The final run is the retained baseline.

Criterion stores native `benchmark.json`, `estimates.json`, and `sample.json` files under
`target/criterion/bash_rules_c5cc97fdadd1/<case>/pre-optimization-musl/`. A repository-owned copy of those measurements
and run metadata is in [bash-rules-baseline.json](bash-rules-baseline.json), so cleaning `target` does not erase the
baseline evidence. The values below use Criterion's slope estimate for linear sampling and mean for flat sampling, with
95% bootstrap confidence intervals; these are not per-request latency percentiles.

After cleaning `target`, restore the saved Criterion baseline from the repository root before using `--baseline`:

```python
import json
from pathlib import Path

data = json.loads(Path("doc/benchmarks/bash-rules-baseline.json").read_text())
for case in data["cases"]:
    directory = Path("target/criterion") / case["benchmark"]["directory_name"] / data["baseline"]
    directory.mkdir(parents=True, exist_ok=True)
    for part in ("benchmark", "estimates", "sample"):
        (directory / f"{part}.json").write_text(json.dumps(case[part]))
```

This restores evidence from this machine, not a portable absolute performance threshold. With a custom
`CARGO_TARGET_DIR`, restore beneath that directory instead of `target`.

| Case                         | Directly measured function                        |  Estimate |           95% CI |
| ---------------------------- | ------------------------------------------------- | --------: | ---------------: |
| `config/parse_toml`          | `toml::from_slice::<UserConfig>`                  |  1.032 ms |   0.997–1.085 ms |
| `config/read_and_parse`      | `load_user_config_from`                           |  1.055 ms |   1.047–1.063 ms |
| `compile/expand_fragments`   | `expand_fragments`, across all 193 rules          |  6.264 ms |   6.240–6.286 ms |
| `compile/individual_regexes` | `Regex::new`, across all 193 expanded patterns    | 20.539 ms | 20.345–20.798 ms |
| `compile/command_set`        | `RegexSet::new`, 186 command patterns             | 10.667 ms | 10.557–10.796 ms |
| `compile/redirect_set`       | `RegexSet::new`, seven redirect patterns          |  85.87 µs |   85.35–86.49 µs |
| `compile/engine_and_drop`    | `BashRuleEngine::from_config`                     | 41.561 ms | 41.175–42.003 ms |
| `split/simple`               | `split_command`                                   |  1.585 µs |   1.576–1.593 µs |
| `evaluate_warm/simple`       | `BashRuleEngine::evaluate_sync`                   |  2.082 µs |   2.069–2.097 µs |
| `trace/simple`               | `command_trace`                                   |  357.9 ns |   356.6–359.4 ns |
| `json/simple`                | `serde_json::to_string_pretty`                    |  1.814 µs |   1.801–1.826 µs |
| `split/compound`             | `split_command`                                   |  4.258 µs |   4.247–4.274 µs |
| `evaluate_warm/compound`     | `BashRuleEngine::evaluate_sync`                   |  4.972 µs |   4.969–4.975 µs |
| `trace/compound`             | `command_trace`                                   |  568.8 ns |   568.3–569.3 ns |
| `json/compound`              | `serde_json::to_string_pretty`                    |  3.103 µs |   3.102–3.105 µs |
| `split/redirect`             | `split_command`                                   |  3.724 µs |   3.720–3.728 µs |
| `evaluate_warm/redirect`     | `BashRuleEngine::evaluate_sync`                   |  8.954 µs |   8.947–8.959 µs |
| `trace/redirect`             | `command_trace`                                   |  764.0 ns |   762.2–766.6 ns |
| `json/redirect`              | `serde_json::to_string_pretty`                    |  6.398 µs |   6.396–6.400 µs |
| `startup/runtime_default`    | `tokio::runtime::Builder::build`, default workers |  822.4 µs |   804.9–843.0 µs |
| `startup/runtime_one_worker` | `tokio::runtime::Builder::build`, one worker      |  33.65 µs |   33.09–34.28 µs |

`simple` is `ls`; `compound` is `ls && git status`; `redirect` is `ls >/dev/null 2>&1`. Permission mode is absent,
matching an invocation without `--mode`.

### Timing boundaries

- All cases include destruction of their result. Regex and engine cases therefore include deallocation relevant to the
  short-lived CLI. `engine_and_drop` clones its owned config in Criterion's **untimed** per-iteration setup; fixture
  reading, initial parsing, and expected-result assertions are also outside that timer.
- Regex component cases use pre-expanded patterns; they do not charge expansion twice. They are diagnostic batch costs,
  not per-rule averages. Their allocation order/lifetimes differ from full engine construction, so do not sum them into
  an exact reconstruction of `engine_and_drop`.
- `read_and_parse` uses a prebuilt current-thread runtime with a warmed file cache. It includes async filesystem
  dispatch and parsing, but not runtime construction. The runtime cases measure construction **and shutdown** directly,
  not process startup, Clap parsing, tracing initialization, or dynamic-loader work.
- `evaluate_warm` uses a compiled, warmed engine. A new `EvaluationContext` is created within each timed iteration, so
  redirect resolution is not accidentally amortized through a retained filesystem context. It includes splitting;
  `split` is a separate diagnostic view, not an additional cost to add to evaluation.
- `trace` uses an existing evaluation; `json` uses an existing trace. JSON timings exclude stdout writes. First-use
  regex cache initialization and cold filesystem behavior are not measured by the warm evaluation cases. Whole-process
  profiling includes them, but does not isolate them numerically.

## Shared fragment matcher

Retained the module-level `LazyLock<Regex>` for the fixed `{{fragment_name}}` matcher. `FragmentExpander` now accesses
it directly instead of carrying a borrowed matcher field. Fragment definitions, cycle tracking, and expansion counts
remain per call; no policy or expanded-pattern cache was introduced. Removing the pointer shrinks stack-local state, not
a heap allocation.

Compared against the immediately preceding accepted source, `34282a2e`, using the same fixture, machine, and musl target
as above. Preliminary 50-sample runs were noisy, so both versions were remeasured with **200 samples, 3-second warm-up,
and 10-second requested measurement time**. Criterion extended collection where necessary to obtain all 200 samples. The
following are the final module-level implementation's results, not the earlier function-local candidate:

| Case                       |    Before | After, run 1 | After, run 2 |     Time reduction |
| -------------------------- | --------: | -----------: | -----------: | -----------------: |
| `compile/expand_fragments` |  6.197 ms |    275.55 µs |    274.35 µs | 95.6% in both runs |
| `compile/engine_and_drop`  | 40.503 ms |    33.694 ms |    32.390 ms |      16.8% / 20.0% |

The engine change's 95% comparison intervals were −17.21% to −16.41% and −20.40% to −19.65%, respectively. The large
expansion improvement therefore also appeared in the complete engine construction/destruction operation. Matcher
initialization occurs during untimed fixture setup, so these measurements exclude its one first-use compilation in a
fresh process. They are not whole-CLI latency measurements.

The longer run also checked individual regex construction, redirect-set construction, compound splitting/evaluation,
redirect splitting/tracing, and default runtime startup. Criterion reported about +4.2% for the isolated
individual-regex batch (20.516 to 21.370 ms) and about +2% for compound splitting/evaluation (warm evaluation: 4.761 to
4.845 µs). These are unconfirmed differences, not established code-caused regressions: the same final executable's
engine estimate varied by about 3.9% between runs, and within-run significance does not control all between-run
variation. Retention is based on the repeatable aggregate loading gain, not a claim that every microbenchmark improved
or that smaller effects have been ruled out. No allocation reduction was inferred from timing alone.

To reproduce the primary comparison, record a separate baseline on `34282a2e`, then compare from the optimized revision:

```bash
# Before the change:
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  'compile/(expand_fragments|engine_and_drop)' --sample-size 200 --warm-up-time 3 \
  --measurement-time 10 --save-baseline pre-fragment-cache-long-musl

# After the change; repeat this command to check stability:
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  'compile/(expand_fragments|engine_and_drop)' --sample-size 200 --warm-up-time 3 \
  --measurement-time 10 --baseline pre-fragment-cache-long-musl
```

The longer comparison's generated files live under
`target/criterion/bash_rules_c5cc97fdadd1/<case>/pre-fragment-cache-long-musl/`; unlike the original archived baseline,
these local files do not survive cleaning `target`. The original `pre-optimization-musl` archive was not replaced.

## Lazy-DFA cache experiment

Abandoned a `RegexBuilder::dfa_size_limit(0)` experiment for validation-only regexes. Modify captures and matching
RegexSets retained their original settings. Against step 1 (`1266b800`), with 200 samples, 3-second warm-up, and
10-second measurements, the individual-regex batch went from 20.410 ms to 19.478 / 19.809 ms. Full engine
construction/destruction went from 32.438 ms to 31.403 / 32.178 ms. The repeat's 0.8% aggregate reduction was within
Criterion's noise threshold, so the experiment failed the retention gate and its code/benchmark changes were abandoned.
All 1,720 tests had passed, but correctness alone was not grounds to keep an unproven optimization.

## NFA-only validation

Retained a second experiment against the same step-1 source and comparison baseline. Non-Modify patterns are validated
with forward and reverse Thompson NFAs through the already-transitive `regex-automata` 0.4.14 dependency, now also a
direct dependency. Validation mirrors `Regex::new`'s UTF-8 syntax, capture-state accounting, and 10 MiB NFA limit;
reverse construction matters because it can exceed the limit even when the forward NFA fits. Failed probes fall back to
`Regex::new`, preserving its diagnostics and literal-fast-path acceptance. NFAs are discarded after validation.

Only `CommandAction::Modify` retains a full capture regex. Command and redirect RegexSets remain unchanged. Debug checks
compare each set's indexed pattern with the corresponding metadata, and Modify still requires its capture match to
succeed. The benchmark fixture has no Modify rules; setup now asserts this so the isolated `individual_regexes` case can
call the same validator as the engine. That case now measures validation and disposal, not accumulation of a vector of
full Regex objects. No speedup is claimed for capture-heavy policies absent from this fixture.

Both versions used 200 samples, 3-second warm-up, and 10-second requested measurements on the same musl target and
fixture. The final repeat confirms the aggregate gain beyond the isolated validation improvement:

| Case                         |    Before | After, run 1 | After, run 2 | Time reduction |
| ---------------------------- | --------: | -----------: | -----------: | -------------: |
| `compile/individual_regexes` | 20.410 ms |    13.892 ms |    13.696 ms |  31.9% / 32.9% |
| `compile/engine_and_drop`    | 32.438 ms |    27.370 ms |    27.050 ms |  15.6% / 16.6% |

The engine change's 95% comparison intervals were −16.17% to −15.03% and −17.14% to −16.03%. All three warm-evaluation
cases remained within noise or improved under Criterion's comparison. The unchanged command-set construction case was
+5.4% in this run; as with the earlier small differences, this is recorded rather than treated as proof of a causal
regression or dismissed as proven noise. The aggregate gain includes the set construction cost.

```bash
# On step 1 (1266b800), preserve a separate comparison baseline:
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  'compile/|evaluate_warm/' --sample-size 200 --warm-up-time 3 --measurement-time 10 \
  --save-baseline pre-regex-builder-musl

# On the NFA-validation revision:
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  'compile/|evaluate_warm/' --sample-size 200 --warm-up-time 3 --measurement-time 10 \
  --baseline pre-regex-builder-musl
```

The repeat used the same arguments with filter `compile/(individual_regexes|engine_and_drop)`. The baseline name was
first used for the rejected cache-capacity trial but still identifies the unmodified step-1 control. Its generated files
are local to `target/criterion`; neither the original archive nor the fragment-matcher comparison was replaced.

## Command-specific runtime experiment

Abandoned a current-thread Tokio runtime specifically for `test bash-rules`; unrelated commands kept the default
multi-thread runtime. The candidate (`f4773ad8`, abandoned) parsed the CLI before constructing its runtime. During the
trial, `startup/runtime_default` followed that command's selected runtime through a shared constructor; after abandoning
the trial, the benchmark again measures the original multi-thread builder.

The fresh musl control was step 2 (`7a2e4085`), saved as `pre-command-runtime-musl`. Both versions used 200 samples,
3-second warm-up, and 10-second requested Criterion measurements. Runtime construction/destruction fell from **2.204 ms
to 2.788 / 2.762 µs** (about 99.87%); this excludes starting the blocking worker needed by the first async file read.
The unchanged engine case measured 26.888 ms before and 27.454 / 26.997 ms after; its initial +2.1% difference did not
hold on repetition. The first run's simple warm-evaluation difference was +1.7%; compound and redirect changes were
within noise or insignificant. These small differences were not established as causal regressions.

Separate whole-process measurements invoked the real release binary, not the benchmark executable:

```bash
target/x86_64-unknown-linux-musl/release/moriarty test bash-rules \
  -c crates/moriarty/benches/fixtures/bash_rules.toml --json --explain ls
```

Each run warmed that invocation for three seconds, then used Python `timeit.repeat` with `repeat=200, number=2`,
dividing each sample by two. Timed invocations checked the exit code and discarded stdout/stderr; an untimed call
verified the expected `Allowed` result and byte-identical explain output across versions. Process launch, config
reading, compilation, evaluation, output generation, and shutdown are included, so these numbers are not interchangeable
with the warm in-process engine benchmark. Medians below are of the two-invocation sample averages.

| Version           | Run | Mean per invocation | Sample median |
| ----------------- | --: | ------------------: | ------------: |
| Step 2            |   1 |           42.511 ms |     42.055 ms |
| Step 2            |   2 |           43.838 ms |     42.421 ms |
| Runtime candidate |   1 |           41.618 ms |     40.875 ms |
| Runtime candidate |   2 |           44.363 ms |     41.223 ms |

Although the medians improved about 2.8% in both runs, the means did not demonstrate a repeatable whole-command gain.
The user chose to reject the trial rather than retain extra dispatch logic on isolated-startup evidence alone. The
candidate's 1,723 tests had passed, including runtime-flavor coverage for Bash argument/stdin forms and every other
command family, but its code and benchmark changes were abandoned.

For the Criterion comparison, use filter `compile/engine_and_drop|evaluate_warm/|startup/runtime_` with the parameters
above, saving/comparing `pre-command-runtime-musl`; the repeat used `compile/engine_and_drop|startup/runtime_default`.
Generated process samples (`target/bash-runtime-cli-before.json` and `target/bash-runtime-cli-after.json`) and the local
measurement script (`target/measure-bash-runtime-step3.py`) are disposable artifacts and do not survive target cleaning.

## glibc versus musl measurements

Measured the accepted step-2 code (`7a2e4085`, with only documentation added in `f748f0ca`) without changing code or
adding an allocator. Both explicit targets used Rust 1.96.0 / LLVM 22.1.2 and the same fixture. The benchmark inherits
the workspace release profile: optimization level 3, fat LTO, one codegen unit, and debug information. Rustflags and
release/bench optimization overrides checked in the environment were unset. Cargo fingerprints confirmed the same
compiler/profile and empty effective rustflags, and the resolved `regex-automata` 0.4.14 features were identical. The
GNU artifact was dynamically linked to glibc 2.42-61; the musl artifact was static PIE.

Both artifacts were built before timing. Runs were sequential in glibc → musl → musl → glibc order, each with 200
samples, 3-second warm-up, and 10-second requested measurements. The table uses the saved `mean.point_estimate` values,
not the regression slope Criterion may display for short cases. Percentages compare the average of the two run means for
each target; they are not a cross-target Criterion significance test.

| Case                         | glibc run 1 | glibc run 2 | musl run 1 | musl run 2 | Lower glibc time |
| ---------------------------- | ----------: | ----------: | ---------: | ---------: | ---------------: |
| `compile/individual_regexes` |    7.693 ms |    7.811 ms |  13.954 ms |  13.676 ms |            43.9% |
| `compile/command_set`        |    5.924 ms |    5.899 ms |  11.086 ms |  10.879 ms |            46.2% |
| `compile/engine_and_drop`    |   14.517 ms |   14.396 ms |  26.683 ms |  26.721 ms |            45.9% |
| `evaluate_warm/simple`       |    0.928 µs |    0.912 µs |   2.018 µs |   1.999 µs |            54.2% |

The aggregate compilation gap is large and repeatable: glibc took about 14.4–14.5 ms versus musl's 26.7 ms. Its saved
95% mean intervals were 14.492–14.549 / 14.319–14.492 ms, versus 26.535–26.851 / 26.599–26.852 ms for musl.

This establishes a **target/library gap, not an allocator-only result**. Neither build selects a custom global
allocator; both use their system allocator, but they also differ in libc memory-copy/locking implementations,
target-specific standard-library/code generation, and linkage. Criterion's `alloca` C shim is built for each target as
well. Dynamic-loader startup is outside these in-process timing boundaries, so it does not explain this measured gap as
process startup work. No deployment-target or allocator change was made or established as safe by these measurements.

```bash
cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-gnu -- \
  'compile/(individual_regexes|command_set|engine_and_drop)|evaluate_warm/simple' \
  --sample-size 200 --warm-up-time 3 --measurement-time 10 \
  --save-baseline allocator-comparison-gnu-1

cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl -- \
  'compile/(individual_regexes|command_set|engine_and_drop)|evaluate_warm/simple' \
  --sample-size 200 --warm-up-time 3 --measurement-time 10 \
  --save-baseline allocator-comparison-musl-1
```

Repeat musl with baseline suffix `-2`, then GNU with suffix `-2`, to reproduce the run order. The four separate
`allocator-comparison-{gnu,musl}-{1,2}` baselines are local to `target/criterion`; none overwrites the archived original
baseline or earlier optimization controls.

## Four-way allocator comparison

Expanded the comparison on the step-2 implementation, based on `c97590cd`, using an **opt-in** `mimalloc` Cargo feature.
At measurement time, normal builds still used the system allocator; the shared `main.rs` declaration selected the same
allocator in the CLI and included benchmark. Deployment wiring was unchanged during measurement; the subsequent Linux
selection is recorded below.

### Allocator and build controls

Selected [mimalloc](https://github.com/microsoft/mimalloc) for its small MIT-licensed integration, cross-thread
allocation design, and static-musl support: relevant to short-lived hooks, parallel log analysis, and long-lived MCP
servers rather than only regex compilation. This is a candidate choice, not evidence that it beats every allocator on
every Moriarty workload. The alternative jemalloc has a larger build/configuration surface and
[musl-specific background-thread limitations](https://docs.rs/tikv-jemalloc-sys/latest/src/tikv_jemalloc_sys/env.rs.html).

The resolved [Rust wrapper is `mimalloc` 0.1.52](https://docs.rs/crate/mimalloc/0.1.52/source/Cargo.toml.orig), with
`libmimalloc-sys` 0.1.49. Its vendored `v3/include/mimalloc.h` identifies **mimalloc 3.3.2**
(`MI_MALLOC_VERSION 30302`). Default features are disabled: no secure mode, tuning, or C `malloc` override. Only Rust's
global allocator changes; C/libc-owned allocations remain on the target libc. The default Cargo dependency graph
excludes mimalloc entirely.

All four CLI/benchmark pairs were prebuilt and copied to distinct paths before timing. They used Rust 1.96.0 / LLVM
22.1.2, the same release profile and fixture, unset Rustflags, and GCC 15.2.0. Both musl cells used a proper musl C
cross-compiler with musl 1.2.5 headers, including for Criterion's `alloca` shim. An initial Rust-only check had silently
compiled mimalloc with host glibc headers via inherited `CC=gcc`; that C artifact was rebuilt with the correct compiler
before any measurement. Preprocessing confirmed the cross headers did not define `__GLIBC__`. Both musl CLI binaries
remained static PIE; both GNU binaries used dynamic glibc 2.42-61.

Runs used forward then reverse cell order: GNU/system → GNU/mimalloc → musl/mimalloc → musl/system, then the reverse.
Each cell ran the seven Criterion cases below, followed by whole-command measurements. There were 200 samples, 3-second
warm-ups, and 10-second requested Criterion measurements; Criterion sometimes extended collection to satisfy its
sampling schedule. No allocator-tuning or preload environment variables were set.

### In-process results

Each cell lists **run 1 / run 2**. Unlike the preceding mean-only table, these are Criterion's displayed central
estimates, which may use regression slopes for short cases. Compilation cases use flat-sampled means. These are fresh
four-way controls, not comparisons against the earlier two-target baselines.

| Case (unit)                       |   musl / system | musl / mimalloc |  glibc / system | glibc / mimalloc |
| --------------------------------- | --------------: | --------------: | --------------: | ---------------: |
| `config/parse_toml` (µs)          | 1035.1 / 1048.2 | 492.18 / 488.39 | 433.79 / 438.23 |  354.55 / 351.47 |
| `compile/individual_regexes` (ms) | 13.663 / 13.959 |   5.917 / 5.717 |   7.785 / 7.624 |    5.217 / 5.067 |
| `compile/command_set` (ms)        | 10.789 / 10.710 |   5.329 / 5.229 |   5.705 / 5.704 |    4.516 / 4.299 |
| `compile/engine_and_drop` (ms)    | 26.610 / 26.934 | 11.541 / 11.424 | 14.203 / 14.033 |   10.030 / 9.647 |
| `evaluate_warm/simple` (µs)       |   2.033 / 2.023 |   1.373 / 1.348 |   0.858 / 0.883 |    0.854 / 0.830 |
| `json/simple` (ns)                | 1745.5 / 1793.2 | 1749.3 / 1737.9 | 280.14 / 280.58 |  303.87 / 302.35 |
| `startup/runtime_default` (ms)    |   1.589 / 1.673 |   1.313 / 1.748 |   1.832 / 1.965 |    0.740 / 1.696 |

Within each target, mimalloc reduced engine construction/drop by **56.6% / 57.6% on musl** and **29.4% / 31.3% on GNU**.
The improvement holds in both runs and musl/mimalloc beats GNU/system on this stage. With mimalloc on both sides, GNU
still takes about 13–16% less time than musl: libc, target code generation, and linkage differences have not vanished.
Changing the allocator also changes memory layout/cache behavior, so these are allocator-selection effects, not a direct
measurement of time spent inside `malloc` alone.

The additional cases do not establish a universal win. GNU JSON serialization was consistently about 8% slower with
mimalloc (roughly 22–24 ns), while musl JSON was approximately unchanged. Runtime construction/drop varied
substantially, including opposite musl allocator rankings across repeats; it does not support a stable startup-stage
claim.

### Whole-command latency and footprint

Each cell used the same real command:

```bash
<prebuilt-binary> test bash-rules -c crates/moriarty/benches/fixtures/bash_rules.toml --json ls
```

A fresh Python process validated the JSON against the other cells, warmed the executable ten times, then timed 200
sequential invocations with `perf_counter_ns`; stdout/stderr went to `/dev/null`, and every exit status was checked. The
measurements include process creation, the existing Tokio runtime, configuration I/O/compilation, evaluation, and
teardown. They are not samples from the in-process Criterion case.

| Cell             | Mean ms, run 1 / 2 | Median ms, run 1 / 2 | Peak child RSS KiB, run 1 / 2 | Stripped binary bytes |
| ---------------- | -----------------: | -------------------: | ----------------------------: | --------------------: |
| musl / system    |    42.436 / 41.990 |      41.052 / 40.430 |               19,992 / 19,804 |            12,826,592 |
| musl / mimalloc  |    30.759 / 25.118 |      30.140 / 23.728 |               41,952 / 41,660 |            13,013,016 |
| glibc / system   |    27.685 / 28.180 |      26.275 / 27.535 |               19,832 / 19,388 |            12,705,680 |
| glibc / mimalloc |    39.962 / 26.648 |      37.661 / 24.871 |               45,508 / 51,892 |            12,896,304 |

Musl's whole-command mean improved in both runs, by about **28% / 40%**. GNU's did not: the first mimalloc run was
slower and the second slightly faster. Machine load was not controlled; the first GNU/mimalloc CLI run began at a
one-minute load average of 5.83 versus 2.80 on repeat. That is a confounder, not proof of the cause or grounds to
discard the slower run. GNU whole-command improvement remains unestablished.

RSS is Linux `getrusage(RUSAGE_CHILDREN).ru_maxrss`: the maximum across each cell's 211 invocations, including
validation and warm-up, not an average or a steady-state server measurement. It can include pre-exec launcher memory, so
the system values may have a launcher-imposed floor. The direct CLI follow-up below supersedes these RSS figures for
footprint decisions and does not reproduce the earlier 41–51 MiB mimalloc readings. Stripped size came from `strip -o`
on separate copies; timing used the original unstripped release binaries. Mimalloc added about 182 KiB on musl and 186
KiB on GNU, roughly 1.5% in either case.

These results support mimalloc as a substantial **musl compilation/CLI speed versus memory trade-off**, not a blanket
whole-application default change. No large JSONL throughput run, long-lived MCP retention/soak test, or other-platform
validation was performed during this comparison. Deployment selection remained undecided until the follow-up below.

### Follow-up: absolute footprint and isolated GNU reruns

All four saved CLI binaries still matched their recorded SHA-256 hashes. No binaries were rebuilt or allocator settings
changed for this follow-up.

For RSS, GNU `time -f '%M'` measured the CLI child directly, rather than using Python's cumulative child-resource
accounting. Each cell ran 20 times, then another 20 in reverse cell order, with the same fixture, command, and
suppressed stdout. Every invocation succeeded. The table shows the **range of per-invocation peak RSS across 40
executions**, not a steady-state heap size or a bound for other workloads. Binary sizes reuse the original build
metadata: decimal MB means 1,000,000 bytes, while RSS MiB means 1,048,576 bytes. Unstripped binaries include
release-profile debug info.

| Cell             | Stripped MB | Unstripped MB | Direct peak RSS range, MiB |
| ---------------- | ----------: | ------------: | -------------------------: |
| musl / system    |      12.827 |       153.340 |                10.51–11.32 |
| musl / mimalloc  |      13.013 |       155.407 |                14.37–19.73 |
| glibc / system   |      12.706 |       154.978 |                11.83–13.06 |
| glibc / mimalloc |      12.896 |       156.920 |                15.57–20.52 |

The largest observed mimalloc peaks were about **8.4 MiB higher on musl** and **7.5 MiB higher on GNU** than their
system controls' largest peaks. These direct measurements do not reproduce the earlier launcher-based 41–51 MiB mimalloc
readings; their discrepancy is not isolated to a proven cause here. The earlier blanket claim that peak RSS roughly
doubles is withdrawn, rather than treating those historical readings as reliable CLI-only memory estimates.

GNU whole-command latency was rerun independently of Criterion and its thread-heavy runtime case, using the original
Python timing mode and prebuilt binaries. Batches ran system → mimalloc → mimalloc → system, with ten warm-ups and 200
measured invocations per batch. This balances batch order; individual invocations were not interleaved. One-minute load
averages remained around 1.1–1.3, and all outputs continued to match the reference JSON.

| Cell             | Mean ms, run 3 / 4 | Median ms, run 3 / 4 |
| ---------------- | -----------------: | -------------------: |
| glibc / system   |    28.423 / 28.518 |      27.754 / 27.510 |
| glibc / mimalloc |    34.890 / 35.800 |      33.394 / 34.845 |

These GNU reruns are consistent, but **favor the system allocator**: mimalloc's whole-command mean is **22.8% / 25.5%
higher**. Its faster warm engine construction/drop does not establish faster cold-process execution. The original GNU
runs are retained above; this follow-up does not erase their variability or prove why one earlier mimalloc run was
faster. The observed GNU CLI regression argues against a blanket allocator switch for this workload.

Direct RSS samples are `target/allocator-four-way/{gnu,musl}-{system,mimalloc}-rss-direct-{1,2}.txt`; GNU rerun samples
are `target/allocator-four-way/gnu-{system,mimalloc}-{3,4}-cli.json`. Only documentation changed for the follow-up: no
code-test/build rerun was necessary for the hash-verified existing binaries. The report and plan were proofread and
formatted; the previously accepted unrelated Semgrep failure and deferred auto-review remain unchanged.

### Reproduction and artifacts

For musl, enter a C cross-toolchain environment before building either allocator cell:

```bash
nix shell nixpkgs#pkgsCross.musl64.stdenv.cc
export CC_x86_64_unknown_linux_musl=x86_64-unknown-linux-musl-gcc
```

Use the following for each explicit target, omitting `--features mimalloc` for its system-allocator control. Save each
pair before building another feature configuration because Cargo reuses the top-level CLI path.

```bash
cargo build --locked --release -p moriarty --bin moriarty --bench bash_rules \
  --target x86_64-unknown-linux-musl --features mimalloc

cargo bench -p moriarty --bench bash_rules --target x86_64-unknown-linux-musl --features mimalloc -- \
  '/(config/parse_toml|compile/(individual_regexes|command_set|engine_and_drop)|evaluate_warm/simple|json/simple|startup/runtime_default)$' \
  --sample-size 200 --warm-up-time 3 --measurement-time 10 \
  --save-baseline allocator-four-musl-mimalloc-1
```

The timed experiment invoked saved benchmark executables directly with `--bench` plus the same Criterion arguments,
without Cargo or compilation inside the measurement sequence. The eight baseline names are
`allocator-four-{gnu,musl}-{system,mimalloc}-{1,2}`, under `target/criterion/bash_rules_c5cc97fdadd1/`. Build commands,
compiler metadata, binary hashes/sizes/linkage, Criterion logs, and individual CLI samples are in
`target/allocator-four-way/`; the disposable driver is `target/allocator-four-way.py`. These local artifacts do not
survive target cleaning. No earlier saved or archived baseline was overwritten.

### Deployment selection

The user selected **static musl with mimalloc for Linux Nix builds**, retaining static portability rather than switching
to dynamic glibc. This accepts the measured musl speed/memory trade-off; it does not establish superiority for
unmeasured long-lived MCP or large log-analysis workloads. `flake.nix` enables `moriarty/mimalloc` for the package,
dependency artifacts, and Linux checks, and sets target-qualified `CC_<target_with_underscores>` to the matching
`pkgsStatic.stdenv.cc` compiler. Host build scripts retain their native compiler. Native Cargo and Darwin builds remain
on the system allocator unless explicitly opted in. No installed system package was changed. The comparison numbers
above still describe the saved Cargo binaries, not a fresh timing of the Nix package.

## Verification

Historical entries below mention the then-existing remediation plan. It was deleted once optimization work finished, per
the repository's plan-retention policy.

The self-contained benchmark smoke run passed all 21 cases; the final musl Criterion run collected all 21 baselines.
Configured Nextest passed 1,720 tests (two skipped), including the shared fixture validation test; all-target Clippy
passed, and Rust/TOML formatting passed. After the Criterion dependency update, `cargo hakari generate --diff` and
`cargo hakari verify` also passed; Nextest was rerun after regeneration. Documentation and the final aggregate changeset
receive the repository's required review separately.

After the shared-matcher change, configured Nextest again passed all 1,720 tests (two skipped); build, all-target
Clippy, and formatting passed. All 21 musl benchmark smoke cases passed. The one-shot CLI
`test bash-rules -c crates/moriarty/benches/fixtures/bash_rules.toml --json --explain ls` returned `Allowed` with the
expected rule attribution.

After NFA validation, configured Nextest passed all 1,722 tests (two skipped), including new parity checks against
`Regex::new` and oversized-pattern rejection for command, redirect, and Modify rules. Build, all-target Clippy, and
Hakari verification passed; no workspace-hack regeneration was needed. All 21 musl benchmark smoke cases passed, and
one-shot CLI explain for `ls >/dev/null 2>&1` returned `Allowed` with both redirect contributors. Auto-review is
deferred until the optimization work is complete.

The glibc/musl comparison built both explicit benchmark targets and passed all 21 smoke cases on each. Only this report
and the remaining plan changed, so no additional code-test run was needed beyond the restored step-2 verification; the
Markdown was proofread and formatted separately.

The four-way experiment passed all 1,722 Nextest tests (two skipped) for each target/allocator combination and all 21
benchmark smoke cases in each cell. An initial musl/mimalloc test run passed with one leaky-test notice; a full repeat
passed without that notice. Configured build, default and feature-enabled all-target Clippy, Rust formatting, and
`cargo hakari generate --diff` / `cargo hakari verify` passed; no workspace-hack update was necessary. The benchmark
fixture and decoded CLI output agreed across all cells; the report and plan were proofread and Markdown-formatted. The
final configured check batch passed formatting, check, Clippy, and Nextest, but Semgrep found six pre-existing
`uncaptured-expect-err` findings in `crates/pi_logs/src/parser/tests.rs`. The user explicitly accepted that unrelated
check failure rather than expanding this task's scope; those assertions were left untouched. Auto-review remains
deferred by the user's instruction.

Static deployment verification built the x86_64 Linux Nix package successfully. `file` and `readelf` confirmed static
PIE with no ELF interpreter or `NEEDED` shared libraries; `nm` showed mimalloc symbols, and a single instrumented
fixture invocation confirmed allocator activity and the expected allow decision plus redirect contributors. That
invocation was an allocator/linkage smoke check, not a timing or RSS measurement. The local package link is
`target/nix-mimalloc`. Nix musl+mimalloc all-target Clippy and Rustdoc passed with warnings denied. Four pre-existing
private Rustdoc links in `crates/pi_logs/src/parser.rs` were converted to inline code with user approval. Nix Nextest
passed all 1,674 selected tests (50 skipped), after the user approved two additions to the existing Nix-only
host-dependency exclusions: one test requires Git, and the other requires named timezone data. The normal configured
suite still passed all 1,722 tests (two skipped), including both excluded-in-Nix tests; configured Clippy, build, and
formatting passed. The Nix/Markdown changes were formatted, preserving the user's pre-existing Markdown emphasis edit.
The aarch64 Linux and both Darwin outputs were evaluated to verify target, compiler, and feature selection, but not
built or executed here. The previously accepted unrelated Semgrep failure remains; no affected assertions changed.
