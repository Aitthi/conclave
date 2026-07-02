---
name: Example Optional Skill
description: Demonstrates the OPTIONAL builtin skill format (mandatory: false) — safe to remove or replace.
mandatory: false
---

This is an example OPTIONAL builtin skill shipped with Conclave (see
docs/adr/0003-optional-system-skills.md). Unlike `example/SKILL.md`
(mandatory, auto-attached to every agent), a skill with `mandatory: false`
in its frontmatter is not attached anywhere by default — a user must pick
it per agent definition, in the Builder's Skills section, the same way a
custom skill is picked. Its content is still read-only.
