# Feature Specification: Semantic Cluster Graph Layout

**Feature Branch**: `001-graph-cluster-layout`
**Created**: 2026-04-28
**Status**: Draft
**Input**: User description: "Граф-канвас Web UI выглядит как hairball: узлы крупных проектов (например boostix — 453 узла) образуют плотное кольцо/сферу вокруг хаба проекта, без визуально различимых подкластеров. Цель: переработать раскладку так, чтобы дочерние узлы группировались в визуально различимые подкластеры по семантике связей."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Read project structure at a glance (Priority: P1)

As a user opening the graph view, I want each project's child nodes to settle into visually distinct sub-groups by their semantic role (decisions, problems & their solutions, sessions, tasks with their work logs), so that I can read the structure of a project without zooming, panning, or clicking individual nodes.

**Why this priority**: This is the core value of the visualization. If the graph reads as a uniform ring/sphere of dots around each hub (the current state), the view fails its primary purpose — surfacing meaningful structure in the knowledge graph. Without this, the rest of the UI (filters, search, hover) only mitigates a broken first impression.

**Independent Test**: Open the graph at the default zoom on a database that contains at least one project hub with 100+ children of mixed types. Without interacting, a viewer must be able to point to at least two distinct sub-groups within that project's cluster and correctly identify what they contain (e.g., "this clump is the decisions, that one is the sessions"). Passes if labels and grouping make this possible; fails if the cluster reads as a uniform ring or amorphous blob.

**Acceptance Scenarios**:

1. **Given** the graph contains a project with ≥100 children of mixed node types, **When** the user opens the graph for the first time, **Then** within ~3 seconds the layout stabilizes into a state where the project's children form sub-groupings that correspond to topical edges (e.g., a Problem and its Solution are visibly close; a Task and its WorkLogs are visibly close), rather than a uniform ring around the project hub.
2. **Given** the graph contains multiple projects, **When** the user opens the graph, **Then** each project forms its own visually separate "island" — children of different projects do not intermingle in the central area.
3. **Given** the layout has stabilized, **When** the user inspects any cluster, **Then** no two nodes are rendered overlapping (their drawn circles do not occlude each other).

---

### User Story 2 - Stable layout under interaction (Priority: P2)

As a user filtering, searching, or selecting nodes in an already-laid-out graph, I want the spatial arrangement to stay put so that I can build a mental map of where things are. The graph should not violently re-shuffle every time I narrow the project filter or type into search.

**Why this priority**: Once Story 1 is delivered, users invest effort in learning the spatial layout ("the auth decisions live in the upper-left corner of the boostix cluster"). Re-shuffling on every filter change destroys that mental map and makes the graph feel chaotic.

**Independent Test**: Let the graph stabilize. Type a search query that highlights a small subset of nodes. Verify that node positions do not jump — only the visual highlight changes. Then drag a single node and release it. Verify the dragged node settles smoothly back into the simulation without ejecting neighbors across the canvas.

**Acceptance Scenarios**:

1. **Given** the graph has stabilized, **When** the user types in the search box and the highlight set changes, **Then** node positions remain visually stable (no perceptible re-layout shake).
2. **Given** the graph has stabilized, **When** the user drags a node and releases it, **Then** the dragged node returns to free simulation (it is not pinned in place) and settles within ~1 second without causing a visible cascade across the rest of the graph.
3. **Given** the user changes the active project filter, **When** the visible node set changes, **Then** the remaining nodes keep their established relative positions.

---

### User Story 3 - Tunable physics without code changes to the renderer (Priority: P3)

As a developer iterating on visual quality, I want all layout-physics parameters (forces, distances, decay rates, relation-to-distance mappings) to live in a single, named, well-commented configuration module — separate from the rendering component — so that I can tune the look-and-feel without reading or risking regressions in 300 lines of canvas drawing code.

**Why this priority**: Lowers the cost of future tuning iterations (Story 1 is unlikely to be perfect on first pass) and makes the layout's intent legible to anyone reviewing the code later. Not user-visible, but a quality-of-life and maintainability requirement.

**Independent Test**: A developer unfamiliar with the file can locate every layout parameter in one file, change one value (e.g., the tether length for `belongs_to`), and observe the effect on screen — without editing the canvas component file.

**Acceptance Scenarios**:

1. **Given** the codebase, **When** a developer searches for "physics" or "force", **Then** they find a single dedicated module containing every layout parameter, with inline comments explaining what each parameter controls.
2. **Given** that module, **When** a developer changes a single value, **Then** the change is reflected on screen on next reload, with no edits required to the canvas component.

---

### Edge Cases

- **Empty / single-node graph**: Layout must not throw or oscillate when there are zero nodes, one node, or zero edges.
- **Project hub with no children**: A bare project node should sit calmly somewhere on canvas, not jitter or fly off.
- **Disconnected components**: Nodes belonging to no project, or projects with no children, must remain on-screen — not drift to infinity. (The current "no center force" tweak risks this.)
- **Very large hub** (e.g., boostix at 453 children today, growing): Layout must reach a stable state in bounded time and not freeze the UI thread.
- **Dragging a node fast**: Releasing must not pin the node or send it across the canvas at high velocity.
- **Hot-reload during dev**: Changing physics parameters and saving the file must take effect on next render without requiring a hard refresh of the browser.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The layout MUST treat the structural relation `belongs_to` (project hub → child) as a long, soft tether, distinct from topical relations.
- **FR-002**: The layout MUST treat topical relations (`contains`, `subtask_of`, `solves`, `caused_by`, `blocks`, `related_to`, `uses`, `depends_on`) as short, strong attractions that visibly cluster the connected nodes.
- **FR-003**: The layout MUST prevent rendered nodes from visually overlapping, regardless of how many siblings share a single hub.
- **FR-004**: The layout MUST reach a visually stable state (no further perceptible motion) on a graph of approximately 1,200 nodes and 2,300 edges within 3 seconds of opening the view on a typical developer workstation.
- **FR-005**: The layout MUST preserve node positions when the visible/highlighted set changes due to filtering, searching, or selection — i.e., the underlying simulation MUST NOT re-heat on those events.
- **FR-006**: The layout MUST re-heat the simulation only when the structural set of nodes or edges changes (nodes added, removed, or re-fetched).
- **FR-007**: When the user drags a node and releases it, the layout MUST return the node to free simulation (no pinning) and reach stability without ejecting neighbors.
- **FR-008**: All layout-physics parameters MUST live in a single dedicated module, separate from the rendering/drawing component, with inline documentation describing each parameter's effect.
- **FR-009**: The mapping from relation type to layout parameters MUST be data-driven: adding or changing a relation's distance/strength MUST require editing only the configuration module, not the renderer.
- **FR-010**: Disconnected nodes and disconnected components MUST remain within the visible canvas region and MUST NOT drift indefinitely outward.
- **FR-011**: The temporary working artifact `graph-after-physics-tweak.png` in the repository root MUST be removed.

### Key Entities

- **Layout Parameter Set**: The single source of truth for all forces governing node arrangement (per-relation distance and strength, repulsion strength and range, centering pull, collision padding, simulation decay/cooldown). Tuned for hub-and-spoke topology with topical sub-clustering.
- **Relation–Force Mapping**: A table associating each known relation type with its layout role — "structural tether" (long, soft) vs. "topical attractor" (short, strong) — plus a default for unmapped relations.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: On a graph containing the boostix project (453 children of mixed types), an unprompted viewer can identify at least 2 distinct sub-clusters within the project's cluster within 10 seconds of opening the graph (vs. 0 distinguishable sub-clusters in the current state).
- **SC-002**: The layout reaches its stable state within 3 seconds of opening the view on a graph of ~1,200 nodes / ~2,300 edges.
- **SC-003**: 0 visually overlapping node pairs in the stabilized layout, on the boostix project at default zoom.
- **SC-004**: 0 perceptible position changes among non-matching nodes when the user types into the search box on a stable layout.
- **SC-005**: A developer modifying a single layout parameter touches exactly 1 file (the layout configuration module) — verified by inspecting the diff of a tuning change.
- **SC-006**: All disconnected nodes and components remain inside the visible canvas region (≤ 10× the initial viewport extent from origin) at all times after stabilization.

## Assumptions

- The visualization remains 2D (no 3D mode in scope).
- No backend changes are required: the existing relation taxonomy (`belongs_to`, `contains`, `subtask_of`, `solves`, `caused_by`, `blocks`, `related_to`, `uses`, `depends_on`, etc.) is sufficient to drive sub-cluster formation.
- Level-of-detail / aggregation (collapsing a hub's children into a single "cloud" representation when zoomed out) is **out of scope** for this feature and is tracked separately.
- Target hardware is a typical developer workstation with hardware-accelerated canvas rendering.
- Existing UX behaviors stay intact: drag-then-release returns nodes to simulation; project labels visible by default while non-project labels appear on hover/selection; existing color and size theming is unchanged.
- Tuning will likely require visual iteration after the first implementation; the spec defines the *quality bar*, not exact numerical force values, since those can only be calibrated against real data.
