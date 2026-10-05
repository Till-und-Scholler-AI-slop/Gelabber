//! Linux monotonic clock shared with the generator's other processes.
//! The bracket bounds clock/Instant conversion, not audio hardware timing.
use crate::archive::Result;
use serde_json::{Value, json};
use std::{ffi::c_long, time::Duration};

#[repr(C)]
struct Timespec {
    seconds: c_long,
    nanos: c_long,
}
unsafe extern "C" {
    fn clock_gettime(clock: i32, time: *mut Timespec) -> i32;
}

pub fn monotonic_ns() -> Result<u64> {
    let mut time = Timespec {
        seconds: 0,
        nanos: 0,
    };
    if unsafe { clock_gettime(1, &mut time) } != 0
        || time.seconds < 0
        || !(0..1_000_000_000).contains(&time.nanos)
    {
        return Err("CLOCK_MONOTONIC read failed".into());
    }
    Ok(time.seconds as u64 * 1_000_000_000 + time.nanos as u64)
}

#[derive(Clone)]
pub struct Anchor {
    pub instant: tokio::time::Instant,
    pub ns: u64,
    pub bracket_ns: u64,
}
impl Anchor {
    pub fn new() -> Result<Self> {
        let before = monotonic_ns()?;
        let instant = tokio::time::Instant::now();
        let after = monotonic_ns()?;
        let bracket_ns = after
            .checked_sub(before)
            .ok_or("nonmonotonic clock bracket")?;
        if bracket_ns > 100_000 {
            return Err("clock conversion bracket exceeds 100us".into());
        }
        // Future start lets every independent source task share this one anchor.
        Ok(Self {
            instant: instant + Duration::from_millis(100),
            ns: before + bracket_ns / 2 + 100_000_000,
            bracket_ns,
        })
    }
    pub fn evidence(&self) -> Value {
        json!({"clock":"CLOCK_MONOTONIC","startClockNs":self.ns.to_string(),"conversionBracketNs":self.bracket_ns,
            "scope":"planned recorded PCM timeline; no hardware, live encoder or acoustic-device latency", "pcm_latency_calibrated":false})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_is_monotonic_and_anchor_future() {
        let before = monotonic_ns().unwrap();
        let anchor = Anchor::new().unwrap();
        let after = monotonic_ns().unwrap();
        assert!(after >= before);
        assert!(anchor.ns > after);
        assert!(anchor.bracket_ns <= 100_000);
        assert!(anchor.evidence()["startClockNs"].is_string());
    }
}
