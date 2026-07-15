# Marrow design-partner outcome study

Component benchmarks show that individual mechanisms work. This study measures whether those
mechanisms improve real work.

## Cohort

- 10–20 developers using at least two agent sessions per week.
- At least five repositories across different sizes and languages.
- Two-week baseline using normal project instructions, followed by four weeks with Marrow.
- Keep agent model, repository, and task mix as stable as practical.

## Primary outcomes

1. Median time from task start to first correct implementation plan.
2. Repeated investigations: code or documentation reopened for a fact already established earlier.
3. Repeated mistakes caused by a forgotten decision or constraint.
4. Conflicting concurrent edits that require manual reconciliation.
5. Useful recall precision, rated by the developer after a warm start.
6. Stale-memory precision: reviewed warnings that identify knowledge requiring action.

## Activation outcomes

- Time from install to first useful warm start.
- Percentage of projects with at least three useful memories after day one.
- Percentage of new users who can explain Project, Hive, Shared, and Channel without assistance.

## Recording

Use [outcomes-template.csv](outcomes-template.csv). Record failed and neutral tasks, not only wins.
Do not include source code, prompts, memory bodies, tokens, or repository names unless the participant
explicitly opts in. Assign random participant and repository IDs.

## Decision bars

Treat a public GA claim as supported only when:

- at least 80% of participants reach a useful warm start within 15 minutes;
- median repeated investigations fall by at least 25%;
- useful recall precision is at least 80%;
- stale-warning precision is at least 90%; and
- collision-related rework does not increase.

Publish the cohort, exclusions, raw anonymized rows, analysis script, and confidence intervals with
any headline result.
