# Current state

This page orients an agent before it searches project memory.
Keep it short; put details in memory events or in the canonical documents.

## Canonical documents

- `README.md`: shipped features, commands, tested behavior and limitations.
- `roadmap.md`: approved direction, milestones and deferrals.
- `glossary.md`: canonical terms. Use them in memory records too.
- `data-model.md`: entities and trust, selection and ownership boundaries.

When memory and these documents disagree, the documents and the code win.
Supersede the stale memory with the correction.

## Memory rules

- `events.jsonl` is append-only. Never edit or delete a line.
- Revise a memory with `memory_supersede` and give the reason.
- Record a memory in the same branch as the change it describes.
- Add `source_refs` (paths, commits, PRs) to findings and decisions.
- `.index/` is generated. `memory_rebuild_index` recreates it.
