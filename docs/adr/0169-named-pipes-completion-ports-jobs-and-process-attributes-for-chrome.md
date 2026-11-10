# 0169: Named pipes, I/O completion ports, job objects and process attribute lists (the IPC Chrome for Windows needs)

**Status:** Accepted. Builds on [0065](0065-nt-threads-synchronisation-and-apcs.md)
(waits, APCs), [0067](0067-nt-file-io.md) (file objects, IRPs, the worker and
the delivery of completions), [0079](0079-our-kernelbase-and-kernel32.md)
(`kernelbase`, `kernel32`), [0089](0089-createprocess-handle-inheritance-startupinfo-create-suspended.md)
(handle inheritance, `CREATE_SUSPENDED`) and [0155](0155-chrome-for-windows-bring-up-stubs-lazy-images-and-the-stage-ladder.md)
(Chrome's stage ladder; its next wall after `version.dll` is named pipes).

## Context

Chrome for Windows is one program in fifteen to twenty processes. They talk
over named pipes (Mojo, Crashpad) with overlapped I/O on completion ports;
the browser starts its sandboxed children with `STARTUPINFOEXW` attribute
lists (handle list, mitigation policy, job list), puts them in job objects,
opens processes by id, duplicates handles across processes and maps
sections into other processes. The spec agent `spec-win-ipc` recorded all
of that on real Windows (49 entries, 55 visible tests); this ADR is the
implementer's side. Inputs: the spec sheets and visible tests, Microsoft
Learn, and the public SDK headers named in the code.

## Decision

### Named pipes: `personalities/nt/src/io/npfs.rs`

`\Device\NamedPipe` is a device object reached as `\??\pipe` (the
`\\.\pipe\` of Win32). A pipe is a name; its instances are made by
`NtCreateNamedPipeFile`; each instance has a server file object and, once
connected, a client file object, two data queues, and one of four states
(listening, connected, closing, disconnected: the module documentation has
the transition table and the statuses of each end in each state, all from
the spec sheets). A write that fits the receiving quota completes at once;
a larger one stays pending as a queue entry that owns its request and
completes when the reader has taken all of it; a zero quota makes every
write wait for a reader. Message pipes keep write boundaries; a short
message read returns `STATUS_BUFFER_OVERFLOW` and leaves the rest. Each end
has its own read mode and completion mode (`FILE_PIPE_INFORMATION`);
duplicates share them. Pipe control codes are `NtFsControlFile`'s
(`FSCTL_PIPE_LISTEN`, `_DISCONNECT`, `_PEEK`, `_TRANSCEIVE`, `_WAIT`, from
`winioctl.h`/`ntifs.h`); `FSCTL_POLY_PIPE_PROCESSES` (0x110100) is Poly's
own, for the process ids `GetNamedPipeClientProcessId` and friends report.
On an asynchronous handle a control code that succeeds at once returns
`STATUS_PENDING` and completes through the usual machinery, as the sheet
of `NtFsControlFile` says. `FSCTL_PIPE_WAIT` polls once per millisecond in
the service. `\Device\PolyPipe` (anonymous pipes, ADR 0093) is unchanged.

### Completion ports: `io/iocp.rs`, `ObjectType::IoCompletion`

A port is an opaque, waitable object signalled while its queue is not
empty. Packets come from `NtSetIoCompletion` and from requests on file
objects associated with the port (`FileCompletionInformation`). The
packet is queued after the caller's buffer and status block are written
(`IoState::finish`), with the key of the association, the request's
`ApcContext` (kernel32 passes the `OVERLAPPED`; NULL when `hEvent`'s low
bit is set), the status and the byte count. No packet for a request with
an APC routine, for one that fails inside the call, or, with
`FILE_SKIP_COMPLETION_PORT_ON_SUCCESS`, for one that succeeded at once.
Requests on a synchronous file object ignore an APC routine; a file object
holds a pointer reference on its port. `NtRemoveIoCompletion(Ex)` wait on
the port object (alertable for `Ex`).

### Jobs: `job.rs`, `ObjectType::Job`

Jobs record their limits and enforce the active-process limit, the
breakaway flags and kill-on-job-close; they count processes, post
`JOB_OBJECT_MSG_*` packets to their port, and a process can be in several
jobs (a hierarchy is flattened). Termination of other processes and the
kill queue are carried out by the kernel after the call
(`sched::terminate_process_by_id`).

### Process attributes

`CreateProcessW` reads the `STARTUPINFOEXW` attribute list (Poly's own
layout, `attrlist.rs`) and passes the handle list, the mitigation policy
(only prohibit-dynamic-code is enforced), the job list and the breakaway
request to `NtCreateUserProcess` in Poly's own attribute blob
(`threads::read_create_attributes`). The handle list restricts inheritance
(`ObjectManager::create_process_with`).

### Cross-process services

`NtTerminateProcess`, `NtMapViewOfSection` and `NtUnmapViewOfSection` work on
other processes (`NtKernel::terminate_other_process`, `with_process`);
`NtQuerySection` exists. `NtOpenProcess` refuses access 0 and the System
process id 4, and honours `OBJ_INHERIT`.

## To be confirmed by the spec agent

The order of two packets when one write completes a waiting read; the
status of a write to a pipe whose server closed (`STATUS_PIPE_CLOSING`
here); a no-wait write that does not fit (`STATUS_PIPE_BUSY` here);
`GetOverlappedResultEx` with an APC when the request is complete; the
job accounting for `TotalTerminatedProcesses`.

## Known gaps

A thread blocked on a port whose last handle closes is not woken with
`ERROR_ABANDONED_WAIT_0`. The main thread of an NT process has the
process id as its thread id (`GetThreadId` of a child's main thread equals
`GetProcessId`; Windows draws both from one table). A copy-on-write view
still reports `PAGE_WRITECOPY` after its first write. Impersonation,
security descriptors of pipes, remote clients and job CPU/memory limits are
not modelled.
