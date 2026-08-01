# Feature Specification: Two-Way Project Sync Between Aurelius Instances

**Feature Branch**: `002-project-sync`

**Created**: 2026-08-01

**Status**: Draft

**Input**: User description: "Two-way sync of a single Aurelius project between two separate Aurelius instances (project owner + an external tester), so both sides automatically see each other's memory graph nodes (decisions, concepts, problems, solutions) and tasks (including bug-report-driven tasks) for that one shared project, without any manual export/import."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Share a project and bootstrap a new collaborator (Priority: P1)

The project owner marks one specific project as shared and grants a collaborator (e.g. an external tester) access to it. The collaborator's own Aurelius instance, on their own machine, receives the complete existing history of that project — every past decision, concept, problem/solution, task, and session summary — without either side manually exporting or importing any file.

**Why this priority**: Without this, there is nothing to build on. A collaborator with an empty or stale picture of the project can't meaningfully participate, and this is the minimum slice that proves data can move between two independent instances at all.

**Independent Test**: Mark a project as shared and grant one collaborator access; on their next session start, their local project snapshot matches the owner's, with no manual steps other than granting access.

**Acceptance Scenarios**:

1. **Given** a project with existing decisions and tasks on the owner's instance, **When** the owner marks that project as shared and grants a collaborator access, **Then** the collaborator's instance shows that project's full existing history at their next session start.
2. **Given** a project that has NOT been marked as shared, **When** any collaborator's instance syncs, **Then** none of that project's data is visible to them.

---

### User Story 2 - See each other's ongoing work automatically (Priority: P2)

While working independently, the owner and collaborator each keep adding decisions, concepts, problem/solution records, and task activity to the shared project. Without either person doing anything beyond normal use of Aurelius, each side's new work shows up on the other side's instance at their next session, tagged with who did it.

**Why this priority**: This is the actual day-to-day value of the feature — the reason to build it at all. It depends on User Story 1 (bootstrap) already working.

**Independent Test**: With sync already bootstrapped, have the owner record a decision and the collaborator record a task on their respective instances; confirm each appears on the other's instance at that person's next session start, correctly attributed to its author.

**Acceptance Scenarios**:

1. **Given** the collaborator adds a new decision or concept to the shared project, **When** the owner starts their next session, **Then** the owner sees that entry along with who created it.
2. **Given** the owner updates or completes a task in the shared project, **When** the collaborator starts their next session, **Then** the collaborator sees the updated status along with who last changed it.
3. **Given** either side is offline or the shared sync point is temporarily unreachable, **When** that person continues working locally, **Then** their local work is unaffected and simply syncs on the next successful attempt.

---

### User Story 3 - Bug report becomes a tracked, attributed task (Priority: P3)

The collaborator (tester) finds an issue and files it as a task against the shared project from their own instance. The owner sees it appear as a new task, clearly marked as filed by the collaborator, picks it up, logs work against it, and marks it done — all of which the collaborator then sees reflected on their side, including who did the work.

**Why this priority**: This is the concrete workflow that motivates the feature (bug reports turning into tracked, resolved work across two people) but is a specific application of User Story 2's general sync behavior, so it can land after the core mechanism works.

**Independent Test**: From the collaborator's instance, file a task; confirm it appears on the owner's instance attributed to the collaborator; have the owner log work and complete it; confirm the collaborator sees the completed status and who completed it.

**Acceptance Scenarios**:

1. **Given** the collaborator creates a task describing a bug on the shared project, **When** the owner next syncs, **Then** the owner sees the new task with the collaborator identified as its author.
2. **Given** the owner logs work against and completes that task, **When** the collaborator next syncs, **Then** the collaborator sees the task marked done along with who completed it and what was logged.

---

### User Story 4 - Deletions and overlapping edits don't corrupt shared history (Priority: P4)

Occasionally someone deletes an obsolete entry, or both people happen to touch the same record around the same time. The system keeps the shared project's history trustworthy in both cases: deletions actually stick on both sides, and an overlapping edit doesn't silently destroy someone's work.

**Why this priority**: These are correctness/edge-case guarantees rather than day-to-day functionality — important for trust in the feature, but the feature delivers value (US1-US3) before this hardening is airtight.

**Independent Test**: Delete an entry on one instance and confirm it is gone on the other instance after the next sync (and does not reappear). Separately, simulate both sides editing the same record before either has synced, and confirm one version wins deterministically while the other remains recoverable rather than silently lost.

**Acceptance Scenarios**:

1. **Given** an entry existing on both instances, **When** it is deleted on one side, **Then** it is also gone from the other side after that side's next sync, and does not reappear from a later sync.
2. **Given** both the owner and the collaborator modify the same record before either has synced the other's change, **When** the next sync happens, **Then** one version is deterministically kept as current and the other is retained in a recoverable form rather than discarded outright.

---

### Edge Cases

- What happens when the shared sync point is unreachable at session start or session end? Local work must proceed normally; the missed sync is simply retried at the next opportunity.
- What happens if the owner revokes a collaborator's access to a shared project? That collaborator stops receiving future updates for that project; data already delivered to their instance is not remotely deleted.
- What happens if a collaborator's local project (by name) already exists locally before sync is enabled for it? Enabling sync must not silently merge it with an unrelated same-named local project without the user's awareness.
- What happens when a third collaborator is added later to an already-shared project? They bootstrap the same way the first collaborator did (User Story 1), without requiring changes to how the owner or existing collaborators work.
- What happens to a project's data on the shared sync point if the project is later un-shared? Existing collaborators stop receiving new updates going forward; already-delivered local copies are unaffected.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The system MUST let the project owner mark an individual project as shared, independent of any other project on the same instance.
- **FR-002**: The system MUST keep projects that are not marked as shared entirely local — never transmitted to or visible from any other instance.
- **FR-003**: The system MUST let the owner grant a specific collaborator access to a specific shared project.
- **FR-004**: When a collaborator is granted access to a shared project, the system MUST deliver that project's complete existing history (all decisions, concepts, problems, solutions, tasks, and session summaries) to the collaborator's instance, not only entries created after access was granted.
- **FR-005**: The system MUST propagate new or changed entries (decisions, concepts, problems, solutions, tasks, task work log activity) made on either side of a shared project to the other side automatically, without a manual export or import step.
- **FR-006**: The system MUST make this propagation resilient to both sides not being active or reachable at the same time — neither side is required to be simultaneously online with the other for sync to eventually succeed.
- **FR-007**: The system MUST require each person to have a durable personal identity (a name and contact identifier) configured once, which is not tied to any single project.
- **FR-008**: The system MUST record, for every entry in a shared project, who originally created it and who most recently modified it, using each person's identity.
- **FR-009**: The system MUST make an entry's creator and last-modifier visible to anyone with access to that shared project.
- **FR-010**: The system MUST propagate deletions made on one side to the other side, so a deleted entry does not persist or reappear on the peer that already received the deletion.
- **FR-011**: The system MUST NOT allow a temporarily unreachable sync point to block or degrade a person's ability to keep working locally, on shared or non-shared projects alike.
- **FR-012**: When the same entry is modified by both sides before their changes have synced with each other, the system MUST deterministically resolve which version is current and MUST retain the non-current version in a recoverable form rather than discarding it silently.
- **FR-013**: The system MUST support adding additional collaborators to an already-shared project without requiring redesign or disruption of existing collaborators' access.
- **FR-014**: Access to a shared project MUST be explicitly granted per collaborator by the owner; the system MUST NOT offer open or self-service enrollment into a shared project.

### Key Entities

- **Project**: A named body of work in Aurelius; gains a "shared" state and an associated set of collaborators once the owner opts it into sync. Non-shared projects are unaffected by this feature.
- **Person (Identity)**: A durable, cross-project identity (name + contact identifier) representing one human, used to attribute every entry they create or modify. Configured once per person, reused across every project they touch.
- **Memory Entry**: Any decision, concept, problem, solution, or session summary belonging to a project — the existing units of Aurelius's knowledge graph. Gains creator/last-modifier attribution and participates in sync when its project is shared.
- **Task**: A trackable unit of work (including bug reports filed as tasks) belonging to a project, with a status and a log of work performed against it. Participates in sync the same way Memory Entries do, including attribution of who filed it and who acted on it.
- **Collaborator Grant**: The record of one person being given access to one shared project, created and revoked by the project owner; determines who bootstraps into and receives ongoing updates for that project.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A newly granted collaborator sees 100% of a shared project's pre-existing decisions and tasks at their very first session after being granted access, with zero manual file transfer steps.
- **SC-002**: An entry created or changed by either side of a shared project is visible to the other side by that person's next session, in at least 95% of syncs performed under normal connectivity.
- **SC-003**: 100% of entries visible through sync display who created them and who last modified them — no synced entry is ever shown without attribution.
- **SC-004**: A deletion made on one side is no longer present on the other side after that side's next sync, in at least 95% of cases, and never silently reappears afterward.
- **SC-005**: Temporary unavailability of the shared sync point results in zero blocked or degraded local operations for either person.
- **SC-006**: Projects not explicitly marked as shared are visible to 0% of other collaborators or instances.
- **SC-007**: Granting a new (e.g. third) collaborator access to an already-shared project requires no changes to how the owner or existing collaborators already work with that project.

## Assumptions

- Sync happens at natural session boundaries (pulling what's new at the start of a session, pushing what changed at the end) rather than continuously in real time; near-instant visibility of a peer's changes is not required.
- A single shared sync point, administered by the project owner, mediates all sync traffic; collaborators do not need to reach each other's machines directly, and this same point can serve additional collaborators later without redesign.
- Collaborator access is provisioned manually by the project owner (e.g., handing a collaborator their credential out of band); no self-service account creation is in scope.
- Concurrent edits to the exact same entry by two people at once are expected to be rare, since normal use is dominated by adding new decisions, tasks, and work log entries rather than both people editing one existing record simultaneously.
- This feature serves the project owner and a small number of personally-invited collaborators (e.g., a tester); it is not intended as a publicly distributed or multi-tenant product, and does not need self-service signup, a management dashboard, or support for an unbounded number of unknown users.
- Automatic field-level merging of conflicting edits (combining both people's changes within a single entry) is out of scope; deterministic last-writer-wins with a recoverable losing version is sufficient for this feature.
- Real-time/live sync (visibility within seconds while both people are simultaneously active) is out of scope; session-boundary sync is sufficient.
