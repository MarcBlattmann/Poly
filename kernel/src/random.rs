//! The kernel's random numbers: one ChaCha20 fast-key-erasure generator
//! (`poly_random::Csprng`, RFC 8439) behind `getrandom(2)`, `/dev/random`,
//! `/dev/urandom` and `/proc/sys/kernel/random`.
//!
//! **Seeding.** The first use seeds it from what the CPU offers
//! (`arch::entropy`): 64 bytes of RDSEED and 64 bytes of RDRAND where the
//! instructions exist, 4096 samples of timestamp-counter jitter, the
//! wall-clock time and the monotonic clock. RDSEED and RDRAND are
//! credited with the full 256 bits; the jitter with 256 bits too, so the
//! generator counts as seeded on CPUs without either (QEMU's default CPU
//! model has neither): `getrandom(2)` never blocks. That is a weaker
//! guarantee than a hardware source on such machines, and logged at boot.
//!
//! **Reseeding.** Once a minute, at the next request, fresh RDSEED or
//! RDRAND output and jitter are mixed in. Bytes written to `/dev/random`
//! or `/dev/urandom` are mixed in at once, without credit (random(4)).

use crate::arch::entropy;
use crate::sync::SpinLock;
use crate::{kprintln, time};
use core::sync::atomic::{AtomicBool, Ordering};
use poly_random::Csprng;
use poly_vfs::Result;
use poly_vfs::device::CharDevice;

struct State {
    rng: Csprng,
    /// When the last reseed was (monotonic nanoseconds).
    reseeded_ns: u64,
}

static RNG: SpinLock<State> = SpinLock::new(State { rng: Csprng::new(), reseeded_ns: 0 });
static SEEDED: AtomicBool = AtomicBool::new(false);

/// How often fresh hardware entropy is mixed in.
const RESEED_NS: u64 = 60 * time::NANOS_PER_SEC;

/// Bytes handed out per lock hold: the lock masks interrupts, so a long
/// request is served in pieces.
const PIECE: usize = 4096;

/// Up to `words` 64-bit values from `source`, as bytes; how many bytes.
fn gather(buf: &mut [u8], source: fn() -> Option<u64>) -> usize {
    let mut n = 0;
    for chunk in buf.chunks_exact_mut(8) {
        match source() {
            Some(v) => {
                chunk.copy_from_slice(&v.to_le_bytes());
                n += 8;
            }
            None => break,
        }
    }
    n
}

/// Fresh input for the generator: hardware output and jitter. Returns the
/// bytes and the bits of entropy to credit.
fn collect(seed: &mut [u8; 512]) -> (usize, u32, &'static str) {
    let mut used = 0;
    let mut credit = 0;
    let mut sources = "jitter";
    let n = gather(&mut seed[..64], entropy::rdseed64);
    if n == 64 {
        credit += 256;
        sources = "RDSEED, RDRAND and jitter";
    }
    used += n;
    let n = gather(&mut seed[used..used + 64], entropy::rdrand64);
    if n == 64 {
        credit += 256;
        if sources == "jitter" {
            sources = "RDRAND and jitter";
        }
    }
    used += n;
    let mut scratch = [0u64; 64];
    let jitter_bytes = 256;
    let mut jitter = [0u8; 256];
    // 16 samples per output byte, folded.
    for round in 0..16 {
        let mut sample = [0u8; 256];
        entropy::jitter(&mut sample, &mut scratch);
        for (j, s) in jitter.iter_mut().zip(sample) {
            *j = j.rotate_left(3) ^ s ^ round;
        }
    }
    seed[used..used + jitter_bytes].copy_from_slice(&jitter);
    used += jitter_bytes;
    credit += 256;
    let clocks = [time::now_ns(), time::realtime_ns(), entropy::tsc()];
    for c in clocks {
        seed[used..used + 8].copy_from_slice(&c.to_le_bytes());
        used += 8;
    }
    (used, credit, sources)
}

/// Seed the generator if nothing has yet.
fn ensure_seeded() {
    if SEEDED.load(Ordering::Acquire) {
        return;
    }
    // Collected without the lock (it masks interrupts; jitter wants them).
    let mut seed = [0u8; 512];
    let (len, credit, sources) = collect(&mut seed);
    let mut st = RNG.lock();
    st.rng.add_entropy(&seed[..len], credit);
    st.reseeded_ns = time::now_ns();
    poly_random::wipe(&mut seed);
    if !SEEDED.swap(true, Ordering::AcqRel) {
        drop(st);
        kprintln!("random: ChaCha20 generator seeded from {sources}");
    }
}

/// Fill `buf` with cryptographically strong random bytes. Never blocks.
pub fn fill(buf: &mut [u8]) {
    ensure_seeded();
    let due = time::now_ns().saturating_sub(RNG.lock().reseeded_ns) > RESEED_NS;
    if due {
        let mut seed = [0u8; 512];
        let (len, _, _) = collect(&mut seed);
        let mut st = RNG.lock();
        // Credit nothing more: the count is full already.
        st.rng.mix(&seed[..len]);
        st.reseeded_ns = time::now_ns();
        drop(st);
        poly_random::wipe(&mut seed);
    }
    for piece in buf.chunks_mut(PIECE) {
        RNG.lock().rng.fill(piece);
    }
}

/// Mix bytes in without crediting entropy (writes to `/dev/urandom`).
pub fn mix(data: &[u8]) {
    ensure_seeded();
    for piece in data.chunks(PIECE) {
        RNG.lock().rng.mix(piece);
    }
}

/// Bits of entropy the generator counts (`entropy_avail`).
pub fn entropy_avail() -> u32 {
    ensure_seeded();
    RNG.lock().rng.credited_bits()
}

/// The boot's random id (`/proc/sys/kernel/random/boot_id`), made on the
/// first request.
pub fn boot_id() -> [u8; 16] {
    static ID: SpinLock<Option<[u8; 16]>> = SpinLock::new(None);
    if let Some(id) = *ID.lock() {
        return id;
    }
    let mut id = [0u8; 16];
    fill(&mut id);
    *ID.lock().get_or_insert(id)
}

/// `/dev/random` (1, 8) and `/dev/urandom` (1, 9), random(4): reads return
/// the generator's output (at most 512 bytes per read from `/dev/random`,
/// 32 MiB from `/dev/urandom`, as random(4) gives); writes are mixed in.
/// Neither ever blocks: the generator is seeded before the first process.
#[derive(Debug)]
pub struct RandomDevice {
    /// The most bytes one read returns.
    max_read: usize,
}

impl RandomDevice {
    /// `/dev/random`.
    pub const RANDOM: RandomDevice = RandomDevice { max_read: 512 };
    /// `/dev/urandom`.
    pub const URANDOM: RandomDevice = RandomDevice { max_read: 32 << 20 };
}

impl CharDevice for RandomDevice {
    fn read(&self, _: u64, buf: &mut [u8], _: bool) -> Result<usize> {
        let n = buf.len().min(self.max_read);
        fill(&mut buf[..n]);
        Ok(n)
    }

    fn write(&self, _: u64, buf: &[u8], _: bool) -> Result<usize> {
        mix(buf);
        Ok(buf.len())
    }
}

// --- Tests ------------------------------------------------------------------

use crate::kernel_test;

kernel_test! {
    fn random_bytes_differ_and_writes_mix_in() -> TestResult {
        let mut a = [0u8; 64];
        let mut b = [0u8; 64];
        fill(&mut a);
        fill(&mut b);
        if a == b || a == [0; 64] {
            return Err("two requests gave the same bytes");
        }
        mix(b"some user input");
        let before = RNG.lock().rng.generation();
        fill(&mut a[..1]);
        if RNG.lock().rng.generation() <= before {
            return Err("the key did not change on output");
        }
        if entropy_avail() < poly_random::READY_BITS {
            return Err("the generator is not seeded");
        }
        if boot_id() != boot_id() {
            return Err("the boot id changed");
        }
        Ok(())
    }
}
