# Skills

Put skill folders here, each containing a `SKILL.md` with frontmatter:

```markdown
---
name: my-skill
description: One line telling the agent when to use this skill.
---

Full instructions the agent reads with read_file when the skill is relevant.
```

Relay lists each skill's name and description in the system prompt. The agent
gets read-only access to skill folders. More locations can be added with
`skills_paths` in config.toml (or `relay migrate`).
