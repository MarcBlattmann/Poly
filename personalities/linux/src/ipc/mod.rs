//! System V IPC (svipc(7)): the objects of an IPC namespace. Shared
//! memory ([`shm`]) and semaphore sets ([`sem`]); message queues are not
//! implemented.

pub mod sem;
pub mod shm;
