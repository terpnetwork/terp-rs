use blake3::Hasher;
use rand::{RngCore, rngs::OsRng};
use std::time::{SystemTime, UNIX_EPOCH};

/// High-quality 32-byte entropy generator with multiple independent sources.
pub fn ultra_secure_random() -> [u8; 32] {
    let mut hasher = Hasher::new();

    // 1. Primary OS entropy (best available system source)
    hasher.update(&get_os_random());

    // 2. Hardware RNG (RDRAND) if available
    // if let Some(rdrand) = get_rdrand() {
    //     hasher.update(&rdrand);
    // }

    // 3. High-resolution timing jitter + CPU cycle noise
    hasher.update(&get_timing_jitter());

    // 4. (Optional) System time + process noise
    hasher.update(&get_process_noise());

    // Optional: Add user entropy if you have it (mouse, keyboard, etc.)
    // hasher.update(&get_user_entropy());

    hasher.finalize().into()
}

fn get_os_random() -> [u8; 32] {
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    buf
}

/// Hardware RNG via RDRAND (x86/x86_64)
// fn get_rdrand() -> Option<[u8; 32]> {
//     let mut buf = [0u8; 32];
//     let mut rng = rdrand::RdRand::new()?;

//     // RDRAND is fast — we can afford multiple calls
//     rng.try_fill_bytes(&mut buf).ok()?;
//     Some(buf)
// }

/// Collect timing jitter + scheduler noise
fn get_timing_jitter() -> [u8; 32] {
    let mut input = Vec::with_capacity(512);

    for _ in 0..150 {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        // Create some unpredictable work
        std::hint::black_box(42u64.pow(9) ^ start as u64);

        let elapsed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();

        input.extend_from_slice(&elapsed.to_le_bytes());
    }

    blake3::hash(&input).into()
}

/// Extra process/system noise
fn get_process_noise() -> [u8; 32] {
    let _buf = [0u8; 32];
    let mut hasher = Hasher::new();

    // Current time
    if let Ok(d) = SystemTime::now().duration_since(UNIX_EPOCH) {
        hasher.update(&d.as_nanos().to_le_bytes());
    }

    // Thread ID
    // hasher.update(&std::thread::current().id().as_u64().to_le_bytes());

    // Memory address of a stack variable (ASLR noise)
    let x = 0u64;
    hasher.update(&(std::ptr::addr_of!(x) as usize).to_le_bytes());

    hasher.finalize().into()
}
