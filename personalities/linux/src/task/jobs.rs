//! Job control rules on the process tree (POSIX "orphaned process group",
//! `_exit`; ADR 0046): which process groups are orphaned, and which
//! become orphaned when a process ends.
//!
//! POSIX: an orphaned process group is "a process group in which the
//! parent of every member is either itself a member of the group or is
//! not a member of the group's session". A group with a member whose
//! parent is in another group of the same session is not orphaned: that
//! parent (a shell) can still continue it. The terminal uses this for
//! `SIGTTIN` / `SIGTTOU` (a background process of an orphaned group gets
//! `EIO` instead of being stopped), and `_exit` says: "If the exit of the
//! process causes a process group to become orphaned, and if any member
//! of the newly-orphaned process group is stopped, then a SIGHUP signal
//! followed by a SIGCONT signal shall be sent to each process in the
//! newly-orphaned process group."
//!
//! Processes that ended and wait to be reaped (zombies) are no longer
//! members, and the kernel (parent 0) is in no session (Poly's reading,
//! to be confirmed by the diff runner).

use super::tree::ProcessTree;
use alloc::vec::Vec;

/// The live members of process group `pgid`.
pub fn members(tree: &ProcessTree, pgid: i32) -> Vec<i32> {
    tree.group(pgid).into_iter().filter(|&pid| !tree.is_zombie(pid)).collect()
}

/// Whether process group `pgid` is orphaned (see the module docs). A group
/// without live members is.
pub fn is_orphaned(tree: &ProcessTree, pgid: i32) -> bool {
    !members(tree, pgid).into_iter().any(|pid| {
        let (Some(parent), Some(sid)) = (tree.parent(pid), tree.sid(pid)) else { return false };
        parent != 0 && !tree.is_zombie(parent) && tree.pgid(parent) != Some(pgid) && tree.sid(parent) == Some(sid)
    })
}

/// Before process `pid` ends: the groups its end may orphan (its own and
/// its children's), each with whether it is orphaned already. Pass the
/// result to [`newly_orphaned`] once the tree has recorded the end.
pub fn before_exit(tree: &ProcessTree, pid: i32) -> Vec<(i32, bool)> {
    let mut groups: Vec<i32> = tree.children(pid).into_iter().filter_map(|c| tree.pgid(c)).collect();
    groups.extend(tree.pgid(pid));
    groups.sort_unstable();
    groups.dedup();
    groups.into_iter().map(|g| (g, is_orphaned(tree, g))).collect()
}

/// The groups of `before` that were not orphaned before and are now.
pub fn newly_orphaned(tree: &ProcessTree, before: &[(i32, bool)]) -> Vec<i32> {
    before.iter().filter(|&&(g, was)| !was && is_orphaned(tree, g)).map(|&(g, _)| g).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::signal::SIGCHLD;
    use crate::task::tree::Termination;

    /// init (1); a shell (10, its own session and group) with a job of two
    /// processes (11 and its child 12, group 11).
    fn shell() -> ProcessTree {
        let mut t = ProcessTree::new();
        t.insert_root(1, 0);
        t.insert_child(1, 1, 10, SIGCHLD, 0);
        t.setsid(10).unwrap();
        t.insert_child(10, 10, 11, SIGCHLD, 0);
        t.setpgid(10, 11, 11).unwrap();
        t.insert_child(11, 11, 12, SIGCHLD, 0);
        t
    }

    #[test]
    fn a_job_of_a_live_shell_is_not_orphaned() {
        let t = shell();
        assert!(!is_orphaned(&t, 11), "the shell (another group, same session) is 11's parent");
        assert!(is_orphaned(&t, 10), "the shell's parent init is in another session");
        assert!(is_orphaned(&t, 1), "init's parent is the kernel");
        assert!(is_orphaned(&t, 99), "no members");
        assert_eq!(members(&t, 11), [11, 12]);
    }

    #[test]
    fn the_shell_ending_orphans_its_jobs() {
        let mut t = shell();
        let before = before_exit(&t, 10);
        assert_eq!(before, [(10, true), (11, false)]);
        t.exit(10, Termination::Exited(0), false);
        assert_eq!(newly_orphaned(&t, &before), [11], "11's parent is now init, in another session");
        // A member of the job ending does not orphan it while the shell lives.
        let mut t = shell();
        let before = before_exit(&t, 12);
        t.exit(12, Termination::Exited(0), false);
        assert!(newly_orphaned(&t, &before).is_empty());
        assert_eq!(members(&t, 11), [11], "the zombie is no member");
    }
}
