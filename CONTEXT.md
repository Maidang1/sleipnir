# Agent Collaboration

This glossary names the roles in Sleipnir's planned agent collaboration
experience. Existing terminal and plugin terminology lives in
[docs/glossary.md](docs/glossary.md).

## Language

**Integrated agent**:
An existing coding agent participating in a workflow coordinated through
Sleipnir's plugin extension. The agent performs the work; the human remains
Sleipnir's user. Each participant runs in its own visible terminal Pane, where
the human can inspect its work and take over interaction.
_Avoid_: Built-in AI engine, Agent identity (which identifies the kind of agent,
not its role in a collaboration).

**Coordinator agent**:
An Integrated agent responsible for starting other agents, assigning their
work, waiting for their results, and synthesizing the outcome. This is a
collaboration role, not a separate AI engine supplied by the plugin.
_Avoid_: Plugin, terminal host.

**Worker agent**:
An Integrated agent carrying out work assigned by a Coordinator agent and
reporting its results back.
_Avoid_: Pane (a location, not a collaboration role).
