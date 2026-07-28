# Feature Specification: Database Durability & Integrity Hardening

**Feature Branch**: `002-db-durability-hardening`
**Created**: 2026-07-28
**Status**: Draft
**Input**: User description: "Durability & integrity hardening для SQLite-слоя Aurelius. Контекст — реальная авария: база %APPDATA%/aurelius/aurelius.db получила \"database disk image is malformed\"; страница-заголовок описывала 181 страницу при 1781 в файле, потому что маленькую БД скопировали файловыми средствами поверх живой WAL-базы при открытых writer-процессах. Данные восстановлены сырым сканом страниц (3077 узлов / 5185 рёбер, integrity ok). Нужно закрыть класс отказа в коде: (1) безопасный многопроцессный доступ; (2) атомарные идемпотентные миграции; (3) ошибка чтения версии схемы не должна интерпретироваться как «новая пустая база»; (4) отказ открывать заведомо повреждённую БД с внятной ошибкой; (5) команды обслуживания `au db check` и `au db backup`; (6) первые тесты в workspace. Единый `db_path()` вместо трёх расходящихся реализаций."

## Context: the incident this feature closes

On 2026-07-27 the user's knowledge graph became unreadable — every Aurelius operation failed with `database disk image is malformed`. Post-mortem of the file itself established:

- The file held 1781 pages of data, but its header declared the database to be 181 pages and marked that size authoritative. Everything past page 181 was invisible to the engine.
- A sibling file, a much smaller database (185 pages, 372 nodes), carried **the same change counter and schema cookie** as the broken file's header — the header belonged to the small database while the body belonged to the large one. Only 14 of the small database's 372 nodes existed in the large one: two genuinely different databases interleaved in one file.

The trigger was an operator copying a small database file over the live one while long-lived writer processes held it open. The engine's cross-process cache coherency does not re-validate the file identity on a running connection, so those writers kept flushing their cached pages into a file that had been swapped underneath them.

Two aggravating factors lived in the product itself and are the actual subject of this feature:

1. **The product offered no safe way to snapshot or restore the database**, so the operator reached for a plain file copy — the one operation that destroys it.
2. **After the damage, every single invocation made it worse.** The code read the schema version, silently converted *any* read failure (including "malformed") into "this is a brand-new empty database", and re-ran the full migration chain — including a destructive step that drops and rebuilds the search index — against live data. Over roughly 24 hours this ran on every CLI call and every hook-triggered write.

Data was recovered by a raw page-level salvage (3077 nodes, 5185 edges, integrity clean). This feature exists so the next occurrence is prevented, detected, or survivable — rather than silent and compounding.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - A damaged database is refused, not silently rewritten (Priority: P1)

As someone whose knowledge graph has just been damaged — by a bad copy, a full disk, a crash, or hardware — I want the tool to stop and tell me what is wrong the first time it notices, instead of continuing to operate on the wreckage and destroying what is still recoverable.

**Why this priority**: This is the difference between "one bad day" and "the data is gone". In the incident, 24 hours of continued operation on a damaged file re-ran destructive schema work hundreds of times. Every other story in this spec is worth less if the tool keeps writing to a file it cannot read correctly. This story alone, shipped by itself, converts an unbounded data-loss event into a stopped, diagnosable one.

**Independent Test**: Take a known-damaged database file (the preserved incident file serves as the fixture). Run any ordinary Aurelius operation against it. The operation must refuse to proceed, must name the specific problem, and must leave the file byte-identical to how it found it.

**Acceptance Scenarios**:

1. **Given** a database whose contents are structurally damaged, **When** any operation opens it, **Then** the operation fails with a message that identifies the file, states that it is damaged, and names the next step the user should take — and no schema changes or row writes are attempted.
2. **Given** a damaged database, **When** the user runs the same operation ten more times, **Then** the file's bytes are unchanged after all ten attempts.
3. **Given** a database whose header describes a smaller database than the file actually contains (the exact fingerprint of the incident), **When** the user asks the tool to inspect it, **Then** the report explicitly calls out the size discrepancy and identifies it as the signature of a file-level copy over a live database.
4. **Given** a healthy database, **When** any operation opens it, **Then** the added safety check costs a negligible, bounded amount of time and the operation proceeds normally.

---

### User Story 2 - A safe way to snapshot the database (Priority: P1)

As a user who wants a backup of my knowledge graph before something risky — an upgrade, a machine migration, a big cleanup — I want a supported command that produces a correct, complete snapshot while the tool is running, so that I never have to reach for a file copy.

**Why this priority**: Equal to Story 1 because it removes the trigger rather than the symptom. The operator in the incident was not being careless — they wanted a backup and the product gave them no way to take one. A documented, one-command snapshot is what makes "never copy the database file" an instruction users can actually follow.

**Independent Test**: With writers actively modifying the database, take a snapshot. The snapshot must open cleanly, pass an integrity check, and contain every record committed before the snapshot began — including records that had not yet been folded back into the main file.

**Acceptance Scenarios**:

1. **Given** the tool is running and actively writing, **When** the user takes a snapshot, **Then** the snapshot completes without interrupting the writers and the source database is left untouched.
2. **Given** a snapshot has been taken, **When** the user inspects it, **Then** it passes an integrity check and its node and edge counts match the source at snapshot time.
3. **Given** the user takes a snapshot without naming a destination, **When** the command completes, **Then** the snapshot is written next to the database under a timestamped name and the path is printed.
4. **Given** the destination file already exists, **When** the user takes a snapshot to that path, **Then** the command refuses and exits non-zero rather than overwriting.
5. **Given** the source database is damaged, **When** the user attempts a snapshot, **Then** the command fails loudly rather than producing a corrupt snapshot that appears successful.

---

### User Story 3 - Verify the database on demand (Priority: P2)

As a user who suspects something is wrong — or who simply wants reassurance after an incident, a crash, or a restore — I want a command that inspects the database and tells me plainly whether it is healthy, without changing anything.

**Why this priority**: Depends on nothing else and is the natural companion to Story 2 (verify the snapshot you just took, verify the file you just restored). It is P2 rather than P1 because Story 1 already blocks the dangerous case automatically; this story serves the user who wants to ask the question themselves.

**Independent Test**: Run the check against a healthy database, against the preserved incident file, and against a snapshot. Each must produce a clear verdict, the check must never modify any of them, and the exit status must distinguish healthy from damaged so it can be used from a script or a hook.

**Acceptance Scenarios**:

1. **Given** a healthy database, **When** the user runs the check, **Then** it reports success, prints the file's size and record counts, and exits zero.
2. **Given** a damaged database, **When** the user runs the check, **Then** it lists the specific problems found, exits non-zero, and points the user at taking a snapshot of whatever is still readable.
3. **Given** any database, **When** the check runs, **Then** the file is not modified and no schema migration is performed — including when the database's schema is older than the current version.
4. **Given** the user wants either a fast answer or an exhaustive one, **When** they run the check, **Then** both a quick mode and a thorough mode are available, with the quick mode as the default.

---

### User Story 4 - Concurrent use does not lose writes (Priority: P2)

As a user running several Claude Code sessions, editor hooks, and the graph viewer at once — all writing to one shared knowledge graph — I want concurrent access to wait its turn instead of failing, so that memories and task logs are not silently dropped when two things happen at the same moment.

**Why this priority**: This is the everyday, non-catastrophic cost of the current design. It does not destroy the database, but it quietly loses individual writes under a workload the product itself creates: an edit hook spawns a writer on every file edit, a stop hook re-indexes the project, the viewer serves requests, and each session runs a server. P2 because losing one memory is recoverable; losing the database is not.

**Independent Test**: Drive many simultaneous writers against one database and assert that every write either succeeds or reports a real error — none may fail merely because another writer held the lock at that instant.

**Acceptance Scenarios**:

1. **Given** multiple processes open the database simultaneously, **When** they all attempt to write, **Then** each waits for its turn up to a bounded timeout and completes, rather than failing immediately on contention.
2. **Given** multiple processes open a brand-new database at the same instant, **When** they each initialize it, **Then** all of them succeed and the resulting schema is correct and initialized exactly once.
3. **Given** a write cannot be completed after waiting, **When** the timeout expires, **Then** the failure is surfaced to the caller rather than discarded.

---

### User Story 5 - An interrupted upgrade never leaves the graph half-migrated (Priority: P3)

As a user upgrading to a new version, I want a schema upgrade to either complete fully or leave my database exactly as it was, so that a crash, a lock conflict, or a power cut during the upgrade cannot leave me with a database that has lost its search index or its constraints.

**Why this priority**: The failure window is narrow (it requires an interruption during an upgrade) and today's upgrades usually succeed. But when it does happen the damage is real and silent — a partially applied upgrade can leave the search index dropped with no record that anything went wrong, and the version marker can advance past work that was never done.

**Independent Test**: Force a schema upgrade to fail partway through, after the destructive step but before the end. The database must afterwards be indistinguishable from its pre-upgrade state, including the recorded schema version.

**Acceptance Scenarios**:

1. **Given** an upgrade fails partway through, **When** the tool is next opened, **Then** the database is in its exact pre-upgrade state and the recorded version has not advanced.
2. **Given** an upgrade succeeds, **When** the tool is opened again, **Then** no upgrade work is repeated and no destructive step re-runs.
3. **Given** a database written by a newer version of the tool than the one being run, **When** an older version opens it, **Then** it refuses with an explanatory message instead of writing to it under an older understanding of the schema.
4. **Given** the schema version cannot be read for any reason, **When** the tool opens the database, **Then** it reports the read failure — it never concludes that the database is new and never runs destructive setup over existing data.

---

### Edge Cases

- **Header/file size mismatch**: a file larger than its own header describes is the incident's fingerprint and must be reported as such, not merely as generic corruption. A file *smaller* than its header describes is only an error when there is no pending write-ahead log to account for the difference.
- **First run, no database yet**: creating a fresh database must still work, must not be confused with the "unreadable version" case, and must be safe when several processes race to do it simultaneously.
- **Read-only or unwritable location**: opening must fail with a message naming the path and the reason.
- **Snapshot destination on a different volume, or with insufficient free space**: must fail cleanly, leaving no partial snapshot presented as complete.
- **Snapshot taken while a large write is in flight**: must produce a consistent point-in-time snapshot, never a torn mixture.
- **The check command run against a file that is not a database at all**: must say so plainly rather than crash.
- **A database being upgraded by another process at the same moment**: the second process waits and then observes the completed upgrade; it must not apply it twice.
- **Damaged database that is still partially readable**: the tool must still allow a snapshot attempt so the user can salvage what is readable, even though ordinary operations are refused.

## Requirements *(mandatory)*

### Functional Requirements

**Integrity gate**

- **FR-001**: Every path that opens the knowledge graph MUST verify the database is structurally sound before performing any write, schema change, or read on behalf of the user.
- **FR-002**: When the verification fails, the system MUST refuse the operation, MUST leave the file unmodified, and MUST report an error that names the file, states that it is damaged, summarizes the problem, and states the recommended next action.
- **FR-003**: The verification MUST detect the case where the file is larger than its own header describes, and MUST report that case with an explanation that it indicates a file-level copy over a live database.
- **FR-004**: The verification MUST add no more than a negligible, bounded cost to opening a healthy database, so that it can run unconditionally on every invocation.

**Safe snapshots**

- **FR-005**: Users MUST be able to produce a complete, consistent snapshot of the database with a single command while the tool and other processes are running.
- **FR-006**: The snapshot MUST include all committed data, including data not yet folded back into the main database file.
- **FR-007**: The snapshot operation MUST NOT modify the source database.
- **FR-008**: When no destination is given, the system MUST write the snapshot beside the database under a timestamped name and print the resulting path.
- **FR-009**: The system MUST refuse to overwrite an existing file when taking a snapshot.
- **FR-010**: A snapshot of a damaged source MUST fail visibly rather than produce a file that appears to be a valid backup.

**On-demand verification**

- **FR-011**: Users MUST be able to run an integrity report on demand, in both a fast default mode and a thorough mode.
- **FR-012**: The report MUST NOT modify the database, MUST NOT perform schema upgrades, and MUST work on databases whose schema is older or damaged.
- **FR-013**: The report MUST print the file size, the record counts, and — when unhealthy — the specific problems found.
- **FR-014**: The command MUST exit zero for a healthy database and non-zero for a damaged one, so it can gate scripts and hooks.

**Concurrent access**

- **FR-015**: Every connection MUST wait, up to a bounded timeout, for a lock held by another process rather than failing on first contention.
- **FR-016**: The system MUST confirm that the intended concurrent-access journaling mode is actually in effect for each connection, and MUST fail with a clear error when it is not, rather than proceeding in a weaker mode.
- **FR-017**: The system MUST NOT weaken existing durability guarantees in the name of throughput.
- **FR-018**: Failed writes MUST be surfaced to the caller. Write failures MUST NOT be silently discarded.

**Schema upgrades**

- **FR-019**: A schema upgrade MUST be atomic: either every step and the version marker are applied together, or nothing is.
- **FR-020**: The system MUST NOT treat a failure to read the current schema version as "this is a new, empty database". Any read failure MUST be reported as such.
- **FR-021**: Applying an upgrade twice MUST be impossible; upgrade steps MUST be safe to attempt on an already-upgraded database and MUST NOT re-run destructive work.
- **FR-022**: The system MUST refuse to operate on a database whose schema is newer than the running version understands, with an explanatory error.
- **FR-023**: Existing databases at the current schema version MUST be usable without any migration work being performed on them.

**Consistency of the database location**

- **FR-024**: All components MUST resolve the database location through a single shared definition, so no component can operate on a different file than the others.

**Guardrails and documentation**

- **FR-025**: User-facing documentation MUST state that copying, moving, or restoring the database file with ordinary file tools while the tool is running will corrupt it, and MUST direct users to the snapshot command instead.

**Verification of this feature**

- **FR-026**: The project MUST carry automated tests covering: concurrent opening of one database, rollback of an interrupted upgrade, refusal of a damaged database, refusal of a newer-than-supported schema, and a snapshot round-trip that includes not-yet-folded-back data. Each test MUST fail against the current code and pass against the delivered change.

### Key Entities

- **Knowledge graph database**: the single local file holding all nodes and edges, shared by every process on the machine. Has a schema version, a structural integrity state, and a physical size that must be consistent with its own header.
- **Snapshot**: a standalone, self-consistent copy of the knowledge graph at a point in time, safe to take while the system is live, and independently verifiable.
- **Integrity report**: a read-only assessment of one database file — a verdict, a list of specific problems, the file's physical geometry, and its record counts.
- **Schema version**: the marker recording which upgrade steps have been applied; must only advance together with the work it describes.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Operating on a damaged database changes zero bytes of it. Verified by hashing the incident file before and after ten consecutive operations.
- **SC-002**: The incident's specific fingerprint — a file larger than its header describes — is reported by name when the tool inspects the preserved incident file.
- **SC-003**: A user can take a verified backup of a live knowledge graph with one command, and confirm it with a second, with no knowledge of the storage engine.
- **SC-004**: A snapshot taken while writes are in flight contains 100% of the records committed before it started.
- **SC-005**: With 8 processes opening and writing to one database simultaneously, 100% of operations either succeed or return a genuine error; none fail merely because another process held the lock.
- **SC-006**: An upgrade interrupted after its destructive step leaves the database byte-for-byte equivalent in content and version to its pre-upgrade state, in 100% of trials.
- **SC-007**: The time added to opening a healthy database by the new safety checks stays under 10 ms for a database of the current size (~7 MB), measured as the difference in wall-clock time to complete a trivial operation.
- **SC-008**: The project goes from 0 automated tests to a suite that reproduces every failure mode listed in FR-026 — each test demonstrably failing before the change and passing after.
- **SC-009**: No user-facing guidance anywhere in the project recommends or implies copying the database file directly.

## Assumptions

- The knowledge graph remains a single local file shared by every process on the machine; introducing a client/server model or per-process databases is out of scope.
- The workload continues to be many short-lived writers plus a few long-lived server processes, all on one machine, with no network filesystem.
- Recovery of an already-damaged database is **out of scope for this feature**. The 2026-07-27 incident was salvaged by an out-of-band page-level scan; productizing that as a repair command is deliberately deferred, because a repair command that silently under-recovers is worse than none. The delivered scope stops at: refuse, report, and snapshot what is readable.
- Restoring a snapshot remains a manual, documented procedure (stop everything, replace, verify) rather than a command, since the dangerous part is the "stop everything" step, which the tool cannot enforce for processes it did not start.
- The existing storage engine and its concurrent-access mode are kept; this feature configures and validates them correctly rather than replacing them.
- Existing databases must keep working untouched: no data migration, no file format change, and a database written by this version must remain readable by the previous version.
- Automated tests may create and destroy temporary databases on the local filesystem.
