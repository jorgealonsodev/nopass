# Phase 10, written but not committed

These files are the app wiring and preflight from M2 phase 10. They were reverted out
of the working tree rather than committed, because one lane B test failed and a red
gate must not land on the branch.

Nothing here is throwaway. The production code looked complete and the failure is
almost certainly in the test's own isolation, not in the wiring. See the "Phase 10 is
waiting in wip/" section of `../RESUME.md` for the diagnosis and how to restore it.

Delete this directory once phase 10 is committed for real.
