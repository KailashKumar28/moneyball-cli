//! Session ids: UTC stamp + a 4-char suffix from a tiny stand-alone
//! PRNG (no rand crate dep) - unique enough for same-second launches.

use chrono::Utc;

/// UTC timestamp + 4-char random suffix so two sessions in the same
/// second don't collide.
pub fn make_session_id() -> String {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    // ONE xorshift state advanced across all four chars. The old code
    // reseeded from the clock per char, so within a microsecond every
    // char was identical (observed live: oooo/3333/rrrr suffixes) -
    // 36 effective ids per second instead of 36^4.
    let mut x = seed();
    let suffix: String = (0..4)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            ALPHA[(x as usize) % ALPHA.len()] as char
        })
        .collect();
    format!("mb-{}-{}", stamp, suffix)
}

// Tiny stand-alone PRNG (no rand crate dep). Good enough for suffix.
const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
fn seed() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::SystemTime;
    // Per-process counter: two ids minted in the same nanosecond tick
    // (or a coarse clock) still get distinct seeds.
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Multiplicative mix (splitmix-style) so counter and clock disperse
    // across all 32 bits - a plain xor left the counter in the high
    // byte only, where a ~16.8ms clock delta could cancel it exactly.
    let mut x = nanos ^ std::process::id().rotate_left(13) ^ n.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    // xorshift needs a non-zero state.
    if x == 0 {
        0x9E37_79B9
    } else {
        x
    }
}
