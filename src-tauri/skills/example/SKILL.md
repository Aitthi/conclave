---
name: Example Skill
description: Demonstrates the builtin skill file format — safe to remove or replace.
---

This is an example builtin skill shipped with Conclave to demonstrate the
file format (see docs/adr/0002-builtin-skills-from-bundled-folder.md). Each
builtin skill is a subdirectory of `skills/` containing exactly one
`SKILL.md` file: two frontmatter fields (`name`, `description`) between
`---` markers, followed by the skill's full instructional content as
ordinary markdown.
