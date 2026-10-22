# Generated Cargo compilation caches

`cargo-boomerang` uses two independent cache identities:

- Generated manifests, sources and reconciled lockfiles are immutable, request-addressed
  workspaces under `<target>/boomerang/generated/v2/<role>/<request>`.
- Compiled dependencies and generated executables share mutable Cargo targets under
  `<target>/b/s<role-prefix><context>`. The context partitions topology, descriptor and
  payload compilation, and includes the Cargo executable, facet-wrapper semantics,
  wrapper paths, and launcher target/profile/toolchain/configuration.

Generated source, dependency selections and source lockfile contents do not partition the
compilation cache. Cargo fingerprints those inputs. Payload compatibility environment
inputs remain tracked through generated `env!` and `option_env!` expressions. An input
change therefore rebuilds affected payload crates while retaining unrelated dependencies.

Facet separation is explicit because the compiler wrapper inserts facet flags after
Cargo constructs its rustc command. A chained user compiler wrapper is also hidden behind
the Boomerang wrapper, so its path participates in the compilation identity.

Each operation validates its own source workspace and the compilation ownership marker,
takes an exclusive compilation-cache lock, and revalidates before running Cargo. Builds
select the exact Cargo package/manifest/binary artifact and make a private executable copy
before releasing the lock. Every generated binary target includes its full request digest;
otherwise Cargo can report a request as fresh while its identically named profile-root
executable belongs to another request. The Cargo binary name uses a single role prefix
(`t`, `d` or `l`) plus the full digest to leave room for MSVC object filenames.
The naming format participates in the source request identity, so older generated
manifests remain separate while dependency compilation targets can still be reused.
Published launchers keep their stable executable
name. Private copies also prevent later builds from changing an executable still in use.

The v2 cache namespace leaves existing v1 workspaces and request-specific build targets
untouched. They are not reused by the new layout.

## Integration harness

Disposable fixture sources remain under
`<target>/tmp/cargo-boomerang-fixtures/<test-binary>/workspace` and are recreated on each
test-binary invocation. Shared compilation targets and deployment output live separately
under `<target>/tmp/cargo-boomerang-builds/<test-binary>/toolchain`. Tests reset their own
deployment output when needed. The explicit cold/warm cache tests continue to allocate
fresh temporary target directories.

Run the generated suite serially, as in CI:

```sh
cargo generate-lockfile --offline
/usr/bin/time -p cargo test --offline --locked -p cargo-boomerang --all-features \
  --test toolchain -- --test-threads=1
```

Repeat the same command to measure a subsequent run. Cargo's `Finished ... target(s)`
line reports the outer tool/test-harness build; the final test summary reports suite
execution, including nested generated Cargo builds. Compare these separately. Registry
downloads are already cached for an offline benchmark; a cold generated compilation
cache does not mean an empty Cargo home.

For an equivalent baseline comparison, use the same compiler and lockfile and a fresh
generated compilation cache on each revision. Existing build directories are intentionally
preserved rather than deleted by this optimization.

## Coverage

Coverage collection discovers Cargo's hashed `deps/l<request>-<cargo-hash>`
executables for all request-specific binary targets. The Linux coverage workflow selects executable
files in those dependency directories, excluding metadata/object files and published
bundles that negative tests may corrupt. A fresh coverage target avoids obsolete
instrumented artifacts from earlier local runs.

## Local benchmark (2026-10-22)

Single serial samples on macOS 26.6.2, aarch64-apple-darwin, rustc 1.99.0
(`b940084d7`, LLVM 23.1.1). Baseline commit:
`688c20f289898850127956b3d566f0a210bb4113`. Both revisions used the same offline
lockfile (SHA-256 `2aa6dad104c4a6f91d72d6d8cb97b0acc802f87b52789e0c2cb48cabdf0f2cb0`).

| Scenario | Suite time | Whole command | Tests |
| --- | ---: | ---: | --- |
| Baseline, cold generated targets | 956.14 s | 977.12 s | 31 passed |
| Shared cache, cold generated targets | 503.22 s | 507.05 s | 31 passed |
| Shared cache, retained targets | 262.74 s | 265.10 s | 31 passed |

The cold suite is 47.4% shorter (1.90x faster). The shared-cache command used
`--show-output` in addition to the command above. Outer tool builds were already
mostly warm; compare suite time for the nested Cargo workload. The new regression
also adds a second launcher variant and an A/B/A freshness check.
The retained-target result is a repeat invocation, not a cold-to-cold comparison;
there is no measured baseline repeat run. It still includes explicitly isolated cold tests.

The A/B/A test reported 115 non-fresh compiler artifacts on the first launcher,
1 on the changed launcher, and 0 when returning to the first launcher. It verifies
both retained executable bytes and the revisited executable, then executes both
variants. Current-build dependency assertions use Cargo artifact messages, including
fresh artifacts, rather than unrelated libraries retained in the shared directory.

Persistent generated targets fell from 58 to 8. Allocated fixture/build storage after
the cold run fell from 48,202,496 KiB to 17,254,164 KiB (about 46.0 to 16.5 GiB).
These are persistent harness roots, excluding invocation-private temporary targets
and the outer workspace build. Disposable sources were recreated normally. The earlier
experimental shared cache was moved aside before the successful cold measurement.
The named-components test clears its own publication output before each run because
its temporary application workspace changes generated file paths between invocations;
its compilation cache remains intact. A preliminary repeat run exposed this conflict
and was excluded from the results, as was an earlier failed implementation benchmark.

These samples precede the CI follow-up shortening binary names; they are not CI
performance guarantees. Windows/Linux execution still requires CI validation.
The follow-up adds a regression test for the Windows runner object-path budget.
Registry downloads were warm, and limited development builds overlapped
the baseline; no other compilation jobs overlapped the final shared-cache benchmark.

Verification: both complete benchmark suites passed 31 tests. Default fast targets
passed 92 tests and all-features fast targets passed 93 (one existing ignored test
in each). Strict all-targets Clippy, package formatting and `git diff --check` passed.
A `cargo llvm-cov --no-report` smoke run of
`codegen::generated_manifest_unions_features_for_one_selected_package` passed; the
collector found one generated launcher object and produced an LCOV report. Cargo's
default report filtering excludes generated source paths under `target/`; this smoke
checks instrumentation and executable discovery, not full CI coverage percentages.
