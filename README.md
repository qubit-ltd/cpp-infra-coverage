# cpp-infra-coverage

![Rust](https://img.shields.io/badge/Rust-1.94-blue)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
![Source version](https://img.shields.io/badge/source-0.1.0-blue)
![Backend](https://img.shields.io/badge/backend-LLVM_18-blue)
![Platform](https://img.shields.io/badge/platform-Linux_x86__64-blue)
[![Language](https://img.shields.io/badge/language-简体中文-blue)](README.zh_CN.md)

[简体中文](README.zh_CN.md)

Measure the C++ implementation actually linked into your tests, reject stale results,
and publish an HTML report with percentage gates. The first supported backend is
Linux x86_64, official Clang 18, Ninja, and LLVM 18 source-based coverage.

```sh
cargo build --locked
cpp-infra-coverage --project /path/to/project plan
cpp-infra-coverage --project /path/to/project --report build/coverage-report.json collect
cpp-infra-coverage --project /path/to/project check --input build/infra-runs/RUN/coverage/CALL/raw.json
cpp-infra-coverage --project /path/to/project report --input build/infra-runs/RUN/coverage/CALL/raw.json
```

The project must declare its scope and a dedicated `coverage` profile in
`.infra/project.toml`. Its committed CMake presets must use Clang 18, Ninja,
`-O0`/`-O1`/`-Og`, compilation flags `-fprofile-instr-generate -fcoverage-mapping`,
and link flag `-fprofile-instr-generate`. Enable `CMAKE_EXPORT_COMPILE_COMMANDS`.
The profile needs configure, build, and test presets. A shared run context may
reference an already prepared absolute `build_context` inside `report_root`; it
is revalidated through `cpp-infra-verify prepare` before the consumption lock is
acquired. Valid reuse does not configure or build again. The consumer then checks
configuration-input hashes and the actual CMake cache digest under the lock and
at final publication. Old contexts without this evidence fail. Dependency
preparation, when configured, is also performed by verify.

Initialization creates `.infra/coverage/config.toml` only when absent, from
`conf/config.toml`; updates do not manage or overwrite project configuration.
Adapt its profile, binary target names and required roots to your project:


```toml
schema = 1
profile = "coverage"
binary_targets = ["example-tests"]
required_source_roots = ["src", "include"]
threshold_exempt_files = []
jobs = 4
timeout_seconds = 300

[thresholds]
lines = 90.0
functions = 95.0
branches = 80.0
# regions = 85.0
```

These percentages are examples, not implicit organization requirements. Approved
organization minima may be set in `.infra/coverage/policy/thresholds.toml` with
`schema = 1` and `[thresholds]`; project configuration may strengthen them.
Unknown TOML fields fail. Scope roots must exist, remain inside the project, and
not overlap. Exact exemptions use `{ path = "src/legacy.cpp", reason = "..." }`;
they remove a file from gate aggregation while preserving it in HTML. All-exempt
or unrepresented required roots fail before the gate is evaluated.

Collection discovers exact objects and owned sources through verify's build
context, holds the shared build lock while consuming them, runs nonempty CTest
suites with an isolated `%m-%p.profraw` pattern, excludes disabled tests from
runnable selection, and verifies actual executed/skipped/failure counts from CTest
JUnit. All-skipped runs fail even if the binary wrote profiles. It merges only this run's profiles,
and exports all selected objects in one LLVM invocation. Missing implementation
mappings (including unlinked static-library sources), invalid profiles, enabled
zero-total metrics, incompatible versions, and changed source/configuration or
object fingerprints are errors. Failed tests remain failed even with sufficient
coverage. Gate totals sum covered/total counts, never file percentages; equality
passes without rounding the displayed percentage.

Each run writes `build/infra-runs/<run>/coverage/<unique-call>/` containing `raw.json`,
`merged.profdata`, original `llvm-export.json`, `summary.json`,
`coverage-badge.json`, and HTML. The common report includes command logs, tool
versions, digests and SHA-256 artifacts; `coverage-html` names a directory using
the canonical directory digest. `check` reuses matching inputs without tests;
`report` renders without introducing a percentage gate and preserves failures
from collection. Both require the original build context, objects, and profdata.

`CPP_INFRA_BIN_DIR` selects the matching verify bundle and never falls back if
its verify executable is missing. `CPP_INFRA_LLVM_COV`, `CPP_INFRA_LLVM_PROFDATA`,
and `CPP_INFRA_CTEST` override external tool paths. LLVM defaults are explicit
`/usr/bin/llvm-cov-18` and `/usr/bin/llvm-profdata-18`. `--offline` propagates to
verify. `--run-context` validates shared project, profile, runtime bundle identity and source/config
digests. C++ entrypoints follow upstream `main` by default; a run uses the actual
runtime bundle without requiring the project's persisted snapshot to match. Only
an explicit infrastructure update changes persisted project snapshot metadata. `--version --json` exposes protocol 1 and a null revision in development
unless `CPP_INFRA_BUILD_REVISION` was set at build time.

Safe project-relative report files outside build are carried as sorted
`source_excluded_paths` in shared contexts, public reports and raw metadata. These
exclude only declared public JSON report files from source hashing. Protected
inputs, unknown existing business JSON, traversal and backslash aliases are
rejected; configuration hashing never uses these exclusions. Project/config/policy
inputs must remain inside the shared digest tracking area, with normal files and
no symlink into build or transaction directories.

Exit codes: 0 passed; 1 tests/build/gates failed; 2 configuration/protocol/input
error; 3 filesystem/spawn/timeout error; 130 cancelled. Child process groups are
terminated on timeout or cancellation. Logs use stderr; machine output uses
stdout only when requested by plan/version.

Header coverage reflects only mappings instantiated by compilation; this does
not measure uninstantiated templates. Functions use LLVM file-level function
counts, retaining separate instantiation counts. Branch coverage is not MC/DC.
AppleClang, MSVC and GCC coverage backends are unsupported.

## Testing

```sh
cargo +nightly-2026-06-05 fmt --all --check -- --config-path .infra/style/rustfmt.toml
CARGO_BUILD_JOBS=2 cargo test --locked
CARGO_BUILD_JOBS=2 cargo clippy --all-targets -- -D warnings
```

Tests use disposable projects and fake process adapters for error boundaries,
plus a real Clang/LLVM 18 test with two instrumented binaries and an uncovered
branch. The real test requires those tools and does not silently skip them.

## License

[Apache License 2.0](LICENSE).

## Contributing

Open an issue or pull request in [the project repository](https://github.com/qubit-ltd/cpp-infra-coverage). Include focused tests for behavior changes.

## Author

Maintained in the [Qubit organization](https://github.com/qubit-ltd). Individual authors are not yet declared in Cargo metadata.
