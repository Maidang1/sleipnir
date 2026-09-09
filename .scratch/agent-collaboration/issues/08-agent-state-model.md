# What do agent state, progress, and completion mean across integrations?

Id: 08
Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: open
Assignee: unassigned
Blocked by: 01, 02, 03, 04

## Question

Which states and progress facts can the plugin truthfully present across the
four adapters? Distinguish process liveness, turn activity, waiting for human
input, task completion, and whether a result has been seen. Decide how uncertain
or stale evidence is represented and what counts as the completion of a specific
delegated task. A shell Run finishing, a quiet screen, and an agent turn ending
must not silently become equivalent facts.
