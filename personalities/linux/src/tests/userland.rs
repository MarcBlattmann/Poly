//! The calls bash, coreutils, Python and gcc needed beyond the earlier
//! milestones (ADR 0057), against [`FakeKernel`].

use super::fake::*;
use crate::{Errno, nr};

// ---- fadvise64 ----------------------------------------------------------

#[test]
fn fadvise64_checks_the_descriptor_the_file_and_the_advice() {
    let mut k = FakeKernel::new();
    k.files.insert(5, File::new(Kind::Regular));
    k.files.insert(6, File::new(Kind::Pipe));
    for advice in 0..=5 {
        assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 0, 0, advice]), Ok(0), "advice {advice}");
    }
    assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 4096, (-1i64) as u64, 2]), Ok(0), "a negative length");
    assert_eq!(sys(&mut k, nr::FADVISE64, &[5, 0, 0, 6]), Err(Errno::EINVAL));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[6, 0, 0, 0]), Err(Errno::ESPIPE));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[9, 0, 0, 0]), Err(Errno::EBADF));
    assert_eq!(sys(&mut k, nr::FADVISE64, &[(-1i64) as u64, 0, 0, 0]), Err(Errno::EBADF));
}
