# Skills

Skill folders in this directory ship with Relay. The installer copies them to
`Program Files\Relay\skills`, and Relay reads them from the `skills` folder next
to `relay.exe` (as well as from `%LOCALAPPDATA%\Relay\skills` and any
`skills_paths` in config.toml). They can be turned off in the app like any other
skill.

Each skill is a folder containing a `SKILL.md` with frontmatter:

```markdown
---
name: my-skill
description: One line telling the agent when to use this skill.
---

Full instructions the agent reads with read_file when the skill is relevant.
```

Relay lists each skill's name and description in the system prompt. The agent
gets read-only access to skill folders.

| Skill | What it does |
| --- | --- |
| [`pcb-design`](pcb-design/SKILL.md) | Circuit-as-code PCB design with KiCad 9 and Freerouting: schematic, layout, autorouting, ERC/DRC, Gerbers/BOM/CPL for JLCPCB, renders and a 3D model. Needs KiCad 9 and Java 25 on the PC. |
