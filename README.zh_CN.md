# cpp-infra-coverage

![Rust](https://img.shields.io/badge/Rust-1.94-blue)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
![Source version](https://img.shields.io/badge/source-0.1.0-blue)
![Backend](https://img.shields.io/badge/backend-LLVM_18-blue)
![Platform](https://img.shields.io/badge/platform-Linux_x86__64-blue)
[![Language](https://img.shields.io/badge/language-English-blue)](README.md)

[English](README.md)

采集实际链接到 C++ 测试程序中的实现覆盖率，拒绝过期结果，并生成 HTML、徽章与百分比门禁。首期支持 Linux x86_64、官方 Clang 18、Ninja 和 LLVM 18 source-based coverage。

```sh
cargo build --locked
cpp-infra-coverage --project /path/to/project plan
cpp-infra-coverage --project /path/to/project --report build/coverage-report.json collect
cpp-infra-coverage --project /path/to/project check --input build/infra-runs/RUN/coverage/CALL/raw.json
cpp-infra-coverage --project /path/to/project report --input build/infra-runs/RUN/coverage/CALL/raw.json
```

项目需在 `.infra/project.toml` 配置源码范围和独立 `coverage` profile。提交的 CMake presets 应使用 Clang 18、Ninja、`-O0`/`-O1`/`-Og`，编译使用 `-fprofile-instr-generate -fcoverage-mapping`，链接使用 `-fprofile-instr-generate`，并启用 `CMAKE_EXPORT_COMPILE_COMMANDS`。profile 必须提供 configure、build 和 test preset。共享运行上下文可通过绝对路径 `build_context` 引用 `report_root` 内已准备的构建；工具先在锁外调用 `cpp-infra-verify prepare` 权威复验，合法复用不重新 configure/build；随后取得消费锁，核对实际配置输入 SHA 与 CMakeCache 摘要，并在最终发布前再次检查。缺少这些证据的旧上下文会失败。配置依赖后，由 `cpp-infra-verify prepare` 完成依赖准备。

初始化仅在缺失时从 `conf/config.toml` 创建项目所有的 `.infra/coverage/config.toml`，正常更新不会管理或覆盖它。请按实际项目适配 profile、二进制 target 名和 required roots：

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

百分比仅为示例，不默认视为组织已经批准的要求。组织可在 `.infra/coverage/policy/thresholds.toml` 通过 `schema = 1` 和 `[thresholds]` 配置已批准的最低要求，项目只能加严。未知 TOML 字段会失败；所有范围目录必须存在，不允许路径越界或范围重叠。阈值豁免采用 `{ path = "src/legacy.cpp", reason = "..." }` 精确路径表，仅从门禁聚合排除，HTML 保留展示。全部豁免或 required root 无 mapping 时直接失败。

collect 从 verify 的构建上下文发现精确对象及项目实现源码，在共享构建锁内消费产物，以隔离的 `%m-%p.profraw` 模式执行非空 CTest suite，选择时排除 disabled 测试，并读取 JUnit 核实实际执行、跳过及失败数；全部跳过时，即使二进制已产生 profile 也会失败。仅合并本次运行数据，并在一次 LLVM export 中导出全部对象。实现源码缺少 mapping（包括静态库对象未链接）、profile 损坏、启用指标总计为零、版本不兼容，或源码/配置/对象指纹变化均会失败。测试失败不会因覆盖率达标而变为通过。指标按 covered/total 求和，不平均文件百分比；等于阈值通过，不使用显示时的舍入值判定。

每次运行在 `build/infra-runs/<run>/coverage/<unique-call>/` 产生 `raw.json`、`merged.profdata`、原始 `llvm-export.json`、`summary.json`、`coverage-badge.json` 和 HTML。公共报告包含命令日志、工具版本、摘要及 SHA-256 制品；`coverage-html` 使用规范目录摘要。check 不重新执行测试，校验后复用结果；report 仅生成可视化，不新增百分比门禁，但保留 collect 的失败状态。两者均需要原构建上下文、对象和 profdata。

`CPP_INFRA_BIN_DIR` 指定 verify 所属工具集，目录缺少对应工具时不会回退到其他 bundle。`CPP_INFRA_LLVM_COV`、`CPP_INFRA_LLVM_PROFDATA` 和 `CPP_INFRA_CTEST` 可覆盖外部工具路径。LLVM 默认显式使用 `/usr/bin/llvm-cov-18` 和 `/usr/bin/llvm-profdata-18`。`--offline` 传递给 verify；`--run-context` 严格校验 project、profile、运行工具集身份和 source/config digest。C++ 项目入口默认追踪上游 `main`，本次运行使用实际 runtime bundle，不要求与项目持久化快照相同；仅显式基础设施更新才改写项目快照元数据。`--version --json` 输出协议 1，开发构建 revision 为 null，除非构建时设置 `CPP_INFRA_BUILD_REVISION`。

项目内 build 外的安全报告文件通过 `source_excluded_paths` 在运行上下文、公共报告和 raw 元数据间传递，仅从源码摘要排除明确声明的公共 JSON 报告文件。受保护输入、已有业务 JSON、路径穿越和反斜杠别名会被拒绝；配置摘要始终不受此列表影响。项目配置和策略必须处于共同摘要可跟踪区，配置文件不允许通过 symlink 导入 build 或事务目录。

退出码：0 通过，1 测试/构建/门禁失败，2 配置/协议/输入错误，3 文件系统/进程启动/超时错误，130 取消。超时或取消会终止子进程组。日志写 stderr，仅 plan/version 等请求的机器输出写 stdout。

头文件仅统计编译实际实例化的 mapping，不计算从未实例化的模板。函数门禁使用 LLVM 文件级 functions，单独保留 instantiations。分支覆盖率不等于 MC/DC。尚不支持 AppleClang、MSVC 或 GCC 覆盖率后端。

## 测试

```sh
cargo +nightly-2026-06-05 fmt --all --check -- --config-path .infra/style/rustfmt.toml
CARGO_BUILD_JOBS=2 cargo test --locked
CARGO_BUILD_JOBS=2 cargo clippy --all-targets -- -D warnings
```

测试使用可丢弃项目与模拟进程适配器覆盖错误边界，并通过真实 Clang/LLVM 18 验证两个插桩二进制、并行隔离 profile 和实际未覆盖分支。真实测试缺少工具时会失败，不会静默跳过。

## 许可证

[Apache License 2.0](LICENSE)。

## 贡献

请在[项目仓库](https://github.com/qubit-ltd/cpp-infra-coverage)提交 issue 或 pull request；行为变更应附带聚焦测试。

## 作者

仓库归属 [Qubit 组织](https://github.com/qubit-ltd)，Cargo 元数据尚未声明个人作者。
