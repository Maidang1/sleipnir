# Who may control an agent, and how does human takeover work?

Id: 05
Parent: ../map.md
Labels: wayfinder:grilling
Type: grilling
Mode: HITL
Status: open
Assignee: unassigned
Blocked by: none

## Question

What authority may a Coordinator agent exercise over Workers, and what must
remain a human action? Decide the intended behavior when human input races with
an automated prompt, an agent asks for approval, or a coordinator wants to stop
or replace a worker. Separate visibility from control ownership and native agent
approval from plugin host grants; do not assume the coordinator can approve its
workers' requests or inject input into unrelated panes.
