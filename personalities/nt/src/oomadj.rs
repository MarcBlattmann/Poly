//! How expendable a Windows process is to the out-of-memory policy (ADR 0250, addendum): the
//! value Linux keeps in `/proc/<pid>/oom_score_adj` (-1000 to 1000, added to the score in
//! thousandths of all memory), from what Windows records about a process rather than its name:
//!
//! - **A child its parent manages.** A process in a job object that another, still running
//!   process made (`NtCreateJobObject`, then `NtAssignProcessToJobObject`) gets
//!   [`MANAGED_CHILD`]. That is how a browser keeps its renderers, GPU and utility processes; when
//!   one of them goes the parent sees it end (the job's port, the process handle) and can start
//!   it again or show the page as crashed, while ending the parent ends them all.
//! - **Memory priority.** `SetProcessInformation(ProcessMemoryPriority)` (Learn: "a lower memory
//!   priority ... the pages are more likely to be trimmed"): every level below
//!   `MEMORY_PRIORITY_NORMAL` adds [`PER_PRIORITY_LEVEL`].
//!
//! The sum is capped at 1000. Nothing makes a Windows process less expendable than 0.

use crate::procattrs::MEMORY_PRIORITY_NORMAL;
use crate::system::NtSystem;
use poly_ntobj::ProcessId;

/// What being in a job another living process made adds.
pub const MANAGED_CHILD: i32 = 500;
/// What each memory priority level below normal adds.
pub const PER_PRIORITY_LEVEL: i32 = 100;

/// The score adjustment of `pid`; `alive` says whether a process is still running.
pub fn score_adj(sys: &NtSystem, pid: ProcessId, alive: impl Fn(ProcessId) -> bool) -> i32 {
    let mut adj = 0;
    if sys.jobs.managed_by_another(pid, alive) {
        adj += MANAGED_CHILD;
    }
    let priority = sys.proc_attrs.get(&pid).map_or(MEMORY_PRIORITY_NORMAL, |a| a.memory_priority);
    adj += (MEMORY_PRIORITY_NORMAL.saturating_sub(priority)) as i32 * PER_PRIORITY_LEVEL;
    adj.min(1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::procattrs::ProcAttrs;
    use poly_ntobj::{NewObject, ObjectType, ThreadRef};

    /// A system with a job made by `creator` holding `member`.
    fn system(creator: u32, member: u32) -> NtSystem {
        let mut sys = NtSystem::new();
        let new = NewObject::Opaque { kind: ObjectType::Job, cookie: 77, signaled: Some(false), parse: false };
        let object = sys.objects.new_object(new, ThreadRef(1)).expect("object");
        sys.jobs.create(77, object, Some(ProcessId(creator)));
        sys.jobs.assign(&mut sys.io, &mut sys.objects, 77, ProcessId(member)).expect("assigned");
        sys
    }

    #[test]
    fn a_child_in_its_living_parents_job_is_expendable() {
        let sys = system(10, 11);
        assert_eq!(score_adj(&sys, ProcessId(11), |_| true), MANAGED_CHILD);
        // The parent itself, and a process in no job: nothing.
        assert_eq!(score_adj(&sys, ProcessId(10), |_| true), 0);
        assert_eq!(score_adj(&sys, ProcessId(12), |_| true), 0);
        // Once the parent is gone the child is on its own.
        assert_eq!(score_adj(&sys, ProcessId(11), |p| p != ProcessId(10)), 0);
        // A process in a job it made itself manages nobody.
        let own = system(11, 11);
        assert_eq!(score_adj(&own, ProcessId(11), |_| true), 0);
    }

    #[test]
    fn a_lower_memory_priority_adds_and_the_sum_stops_at_1000() {
        let mut sys = system(10, 11);
        let mut low = ProcAttrs::new(0);
        low.memory_priority = 1;
        sys.proc_attrs.insert(ProcessId(11), low);
        sys.proc_attrs.insert(ProcessId(12), low);
        assert_eq!(score_adj(&sys, ProcessId(12), |_| true), 4 * PER_PRIORITY_LEVEL);
        assert_eq!(score_adj(&sys, ProcessId(11), |_| true), (MANAGED_CHILD + 4 * PER_PRIORITY_LEVEL).min(1000));
    }
}
