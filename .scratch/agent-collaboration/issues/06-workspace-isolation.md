# How do collaborating agents share or isolate their working files?

Id: 06
Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: open
Assignee: unassigned
Blocked by: none

## Question

When implementation, test, and review Workers act concurrently on one project,
do they share a checkout, use separate worktrees, or choose explicitly per task?
Decide who owns creation, patch handoff, integration, and cleanup, including
uncommitted user work and non-git directories. Keep this a product/workflow
decision rather than instructions to provision directories now.
