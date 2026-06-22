# Layertwine — AGENTS.md

## Language

Always use English in code, comments, logging, error info or other string literal. Use Chinese in docs (except code block)
**Never use any Chinese in any code files or code block.**

## Project

`layertwine` is a lightweight file-edit history storage layer for multi-agent + human collaborative editing. Rust library crate (no binary yet).

**Status:** Early dev — P1 (core types + SQLite) and P2 (diff/merge engine) partially done. P3–P7 are stub `mod.rs` only.

## Build

```sh
cargo test --lib    # run all unit tests (with compile check)
cargo test          # run all tests (unit + integration + e2e)
cargo test --test e2e_tests  # run only e2e tests
```

## Architecture

```
src/
├── core/        # immutable data types — FileNode, Delta, Snapshot, Partition, Layer, types
├── storage/     # SQLite persistence — SqliteStorage, migrations, Repository traits
├── engine/      # diff/merge/inverse — similar-based, three-way merge with conflict detection
├── state_machine/
├── backup/
├── checkpoint/
├── git_sync/
├── cli/
├── lib.rs       # re-exports all modules + pub use error::{LayertwineError, StorageError, StorageResult}
└── error.rs     # LayertwineError + StorageError (thiserror)
```

## Tests

```
tests/
├── common/                         # Shared test infra: fixture, helpers, assertions, output
│   ├── mod.rs
│   ├── fixture.rs                  # TestConfig, TestEnvironment, TestFixture
│   ├── helpers.rs                  # Convenience wrappers (init, edit, commit, approve, etc.)
│   ├── assertions.rs               # Custom assertions for snapshots, partitions, logs
│   └── output.rs                   # Formatted output utilities
├── e2e/
│   ├── mod.rs                      # Declares all 10 e2e sub-modules
│   ├── basic_workflow.rs           # Full edit workflow
│   ├── approval_flow.rs            # Approval workflow
│   ├── backup_restore.rs           # Backup / restore
│   ├── branch_operations.rs        # Branch create / switch / merge
│   ├── checkpoint_restore.rs       # Checkpoint restore / rollback / timeline
│   ├── multi_agent.rs              # Multi-agent collaboration
│   ├── edge_cases_additional.rs    # Edge cases (large files, empty content)
│   ├── error_handling.rs           # Error handling (missing content, invalid branch)
│   ├── maintenance_tests.rs        # Compact, GC
│   └── show_diff_tests.rs          # Show / staged diff commands
├── e2e_tests.rs                    # E2E test binary entry point (mod e2e; mod common;)
├── storage_integration.rs          # Storage layer integration tests
├── engine_integration.rs           # Diff/merge/inverse integration tests
├── engine_test.rs                  # apply_deltas newline handling
├── checkpoint_integration.rs       # Checkpoint / branch / DAG integration tests
├── layered_integration.rs          # Layered state machine pipeline tests
├── backup_integration.rs           # Backup repo integration tests
├── backup_physical_isolation.rs    # Backup physical isolation tests
├── http_integration.rs             # HTTP API integration tests
├── grpc_integration.rs             # gRPC API integration tests (cfg(feature = "grpc"))
└── git_sync_e2e.rs                 # Git sync end-to-end tests
```

**Unit tests:** `#[cfg(test)] mod tests` blocks inside `src/` (core, engine, storage, layered, checkpoint, backup, git_sync, api, cli).
**Shared test helpers in src/:** `src/test_utils.rs` provides `setup_storage()`, `setup_storage_full()`, `create_initial_snapshot()` for `#[cfg(test)]` modules.

## Key patterns

- **Content-addressed IDs:** Blake3 hash of `serde_json::to_vec(self)` for Snapshots, Deltas, Checkpoints
- **Partition IDs:** UUID v7 (time-ordered, B-tree friendly)
- **Storage:** `Repository` trait = `SnapshotStore + DeltaStore + PartitionStore + FileNodeStore`. Implemented by `SqliteStorage` and `LayertwineStorage` (thin wrapper).
- **SQLite:** Open in WAL mode with `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`. Two init paths: `initialize_database` (P1 tables) and `initialize_full` (+ checkpoint/branch tables).
- **Immutable entities are INSERT ONLY:** `file_nodes`, `deltas`, `snapshots` — no UPDATE/DELETE. Mutable state lives in `partitions`, `partition_history`, `layers`.
- **Layers:** `ManualEdit`, `AgentEdit`, `Approval`, `Staged` — partition types mirror these with Agent/Approval being per-Agent-instance-subdivided.
- **Engine diffs:** Uses `similar` crate. `apply_deltas()` applies Delta chain to reconstruct file content. `merge_texts()` does three-way merge with `MergeConflict` result.
- **Inverse deltas** need `old_content` to reconstruct deleted lines (Delete ops don't carry deleted content).

## Dep graph (from docs)

```
P1 → P2 → P3 → P4 → P6 → P7
            ↘         ↗
             P5 ──────┘  → P8
```

## Key deps

| Dep                | Used for                                                          |
| ------------------ | ----------------------------------------------------------------- |
| rusqlite (bundled) | SQLite storage, zero external dep                                 |
| blake3             | Content-addressed hashing (32-byte IDs)                           |
| serde + serde_json | Serialization for ID computation + storage                        |
| similar            | Line-level diff engine (v3.x, current code needs fix for 3.x API) |
| git2               | libgit2 bindings (stub, P6)                                       |
| chrono             | Timestamps for Deltas, Snapshots, Partitions                      |
| uuid v5/v7 + serde | Partition IDs (v7 for new, v5 for deterministic name-based) |

## Other facts

- `ref/` contains reference source archives (git, jj, immer, similar) — gitignored (`ref` in `.gitignore`)
- All docs and comments are in Chinese (`docs/`, inline comments)
- Phase plan: see `docs/plan/00-任务总览.md` and `docs/architecture/`
- No `Cargo.lock` should be committed? It IS committed (present).
- No CI, no formatter config, no linter config — use `cargo fmt && cargo clippy` as defaults
