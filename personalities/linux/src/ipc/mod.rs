//! System V IPC (svipc(7)): the objects of an IPC namespace. Only shared
//! memory so far ([`shm`]); message queues and semaphore sets are not
//! implemented.

pub mod shm;
