# 0203: Job objects nest, and the job information classes answer as the sheets observed

**Status:** Accepted. Builds on [0169](0169-named-pipes-io-completion-ports-job-objects-and-process-attribute-lists.md)
(job objects, which flattened the hierarchy and recorded most limits). The spec sheets of
`NtAssignProcessToJobObject`, `NtQueryInformationJobObject`, `NtSetInformationJobObject`,
`TerminateJobObject`, `OpenJobObjectW`, `QueryInformationJobObject` (sharpened by `spec-win-fixups-1`)
and the visible tests `ntqijo_defaults`, `ntsijo_misc`, `job_terminate_nested`, `qij_priority` are
the inputs; nothing else.

## Context

ADR 0169 gave a process a list of jobs and counted it in each, which was enough for Chrome (one job
per sandboxed child). The sheets of the integrators' runs found the places where Poly's answer
differs from Windows': a process assigned to a job it is not in does not just join it, jobs nest, a
terminate does not count in `TotalTerminatedProcesses`, the information classes accept exactly the
lengths and values the visible scans list, and a job is waitable.

## Decision

- **Nesting** (`Jobs::assign`). A process that is in no job joins the job and the jobs above it. One
  that is in jobs joins the jobs between its innermost one and the new job when the new job is below
  that one; an empty job with no parent is put below the innermost job; any other job refuses it
  with `STATUS_ACCESS_DENIED`. A job the process is already in (or an outer one) changes nothing.
  Every job of the chain counts the process in `TotalProcesses` and `ActiveProcesses`; a terminate
  of a job ends every process of it and of the jobs below it.
- **`TotalTerminatedProcesses` stays 0.** The counter is kept in the structure and never raised.
- **A job is waitable** and signalled once it was terminated (`NtTerminateJobObject`).
- **A process whose first thread never ran** (created suspended, not resumed) leaves no
  `JOB_OBJECT_MSG_EXIT_PROCESS` / `ABNORMAL_EXIT` packet; the `ACTIVE_PROCESS_ZERO` packet stays.
  The kernel tells the job table when it starts the first thread.
- **Query and set tables** (`Job::query`, `set_length`, `parse_set`): the lengths, statuses and
  values of every class the visible scans cover are kept in two tables with unit tests; a set stores
  only the fields its flags select (a later set resets the rest), the basic structure refuses the
  extended flags, and `SchedulingClass` reads 5 unless set.
- **Priority class**: `NtQueryInformationJobObject` holds the kernel's classes 1 to 6 and reads 0
  for a job without a priority limit; `kernel32` reports the `winbase.h` class
  (`NORMAL_PRIORITY_CLASS` for no limit) and converts on the way in
  (`poly_kernelbase::priority::job_priority_to_win32` / `_to_nt`).

## Consequences

- A job tree is exact for the cases the sheets observed. Limits are still not enforced beyond the
  active-process limit, the breakaway flags and kill-on-close (ADR 0169).
- The kernel32 priority conversion is the conventional mapping of the six classes; a value outside
  them is passed to the kernel to judge.
