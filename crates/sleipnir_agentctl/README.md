# sleipnir-agentctl

JSON-lines client for the local agent-coordination socket
(`~/.config/sleipnir/agent-control.sock`, or `--socket` /
`SLEIPNIR_AGENT_CONTROL_SOCKET`).

Launch, prompt, interrupt, focus, and close are **accepted and queued**.
This binary does not spawn an agent, drive a PTY, or consume adapter
effects. Execution is delivered by the built-in Agents plugin, which starts with Sleipnir.

The client is also built into the terminal as `sleipnir agentctl`; no separate
installation is required. Inside a Sleipnir pane, `"$SLEIPNIR_BIN" agentctl list`
works even if the application is not on PATH. All arguments below work with
either entry point. The CLI does not start the GUI or the coordination server.

There is **no** approve command.

```
sleipnir-agentctl list
sleipnir-agentctl launch codex /work --name w1 -- --foo
sleipnir-agentctl launch-wait codex /work --name w1 --timeout-ms 5000 -- --foo
sleipnir-agentctl prompt <session> implement the tests
sleipnir-agentctl prompt-wait <session> --timeout-ms 5000 implement the tests
sleipnir-agentctl wait <task> --timeout-ms 5000
sleipnir-agentctl interrupt <session>
sleipnir-agentctl focus <session>
sleipnir-agentctl inspect <session>
sleipnir-agentctl human-takeover <session>
sleipnir-agentctl close <session>
sleipnir-agentctl effects
sleipnir-agentctl facts [cursor]
sleipnir-agentctl report-running <task>
sleipnir-agentctl report-awaiting-human <task> native dialog
sleipnir-agentctl report-result <task> --stdin
sleipnir-agentctl report-session-closed <session>
```

`report-*` commands record **local worker self-reports**. Any same-user
client of the socket can spoof them. They are not approval answers and
do not carry a success flag.

`wait` polls the registry snapshot on the client until `terminal` is true
or the timeout fires (`--timeout-ms 0`, the default, is one snapshot).
Stdout is one JSON object per poll, including optional `result` and `detail`
from worker reports. IDs are opaque UUIDs taken from responses.
A mismatched response `id` is treated as an error on every round trip.

`prompt-wait` is client-side `prompt` then `wait` on the `prompt_accepted`
task. It prints **one** final JSON object (`op: prompt_wait`) with
`task` / `status` / `result` / `detail` (and `session`). Exit codes match
`wait`. It does not answer native approvals.

`launch-wait` is client-side `launch` then `wait` on the launch task only.
It stops when the worker process is observed (`running` / `awaiting_human`)
or the task is terminal (`failed_delivery` / `unknown` / `settled`). It
prints one JSON object (`op: launch_wait`) with `session` plus the task
fields. It **does not prompt**. Observing `running` is not a success flag.

Exit codes:

| Code | Meaning |
| --- | --- |
| 0 | `wait` `settled`, `launch-wait` process observed, or a non-terminal snapshot (`--timeout-ms 0`) |
| 1 | I/O, protocol, or correlation-id mismatch |
| 2 | usage / unknown command |
| 3 | `wait` ended in `failed_delivery` |
| 4 | `wait` ended in `unknown` |
| 5 | `wait` timed out while still non-terminal |
