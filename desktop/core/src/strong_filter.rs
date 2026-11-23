//! The "strong" microphone filter (GEL-28): DeepFilterNet3 through libDF
//! and tract, handed to the native core as the four functions of
//! `gm_strong_filter`. The core holds the filter (desktop/native/src/
//! strong_filter.h): it builds it off the audio thread, runs it on the
//! APM's mono 10 ms blocks in place of RNNoise's output and falls back to
//! RNNoise when it fails or is too slow.
//!
//! The setup is the one docs/v0.13-deepfilternet-measurement.md measured:
//! the full DeepFilterNet3 model at the pinned commit (libDF embeds its
//! `models/DeepFilterNet3_onnx.tar.gz`), no attenuation limit, the local-SNR
//! thresholds of the `deep-filter` command line (-15/35/35 dB, which run
//! the whole model on every block; libDF's defaults skip stages and rate
//! worse than RNNoise), no post filter.

use std::{
    ffi::{c_char, c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Once,
};

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::{ArrayView2, ArrayViewMut2};

use crate::ffi;

/// What the filter works on: 10 ms at 48 kHz, one APM block.
pub const SAMPLE_RATE: usize = 48_000;
pub const BLOCK_FRAMES: usize = 480;

/// No limit (libDF takes 100 dB and above as none).
const ATTEN_LIM_DB: f32 = 100.0;
/// Local SNR below which libDF outputs silence, and above which it skips
/// the ERB and the deep-filter stage.
const MIN_DB_THRESH: f32 = -15.0;
const MAX_DB_ERB_THRESH: f32 = 35.0;
const MAX_DB_DF_THRESH: f32 = 35.0;
/// libDF logs a warning for every block with a sample above 0.9999, which
/// the app's logger would write from the audio thread.
const PEAK: f32 = 0.9999;

/// One DeepFilterNet3 instance for one mono stream.
pub struct StrongFilter {
    model: DfTract,
    /// The model as built, for a new stream: libDF has no reset of the
    /// network's recurrent state.
    fresh: DfTract,
    enhanced: Vec<f32>,
}

// SAFETY: the core builds a filter on a thread of its own and then runs it
// on the audio thread, one thread at a time. tract's states are not Send
// only because the values between its operators are `Rc<Tensor>`
// (`TValue::Var`) and its operator states are boxed without a Send bound.
// Every one of those `Rc`s lives inside `model` and `fresh` (cloning
// `fresh` shares tensors between the two, never with anything outside
// this value), so they all move to the other thread together and no
// reference count is touched from two threads. tract itself freezes such
// states into `Send` copies for the same move; libDF's `DfTract` keeps
// them private, so this moves them whole. tract's only thread-local is a
// scratch buffer that each thread allocates for itself during a call.
unsafe impl Send for StrongFilter {}

impl StrongFilter {
    /// Builds the model: about half a second, never on the audio thread.
    pub fn new() -> Result<Self, String> {
        let params = RuntimeParams::default_with_ch(1)
            .with_atten_lim(ATTEN_LIM_DB)
            .with_thresholds(MIN_DB_THRESH, MAX_DB_ERB_THRESH, MAX_DB_DF_THRESH);
        let model =
            DfTract::new(DfParams::default(), &params).map_err(|error| format!("{error:#}"))?;
        if model.sr != SAMPLE_RATE || model.hop_size != BLOCK_FRAMES {
            return Err(format!(
                "the model works on {} samples at {} Hz, not {BLOCK_FRAMES} at {SAMPLE_RATE}",
                model.hop_size, model.sr
            ));
        }
        Ok(Self {
            fresh: model.clone(),
            model,
            enhanced: vec![0.0; BLOCK_FRAMES],
        })
    }

    /// Forgets the history: a new stream.
    pub fn reset(&mut self) {
        self.model.clone_from(&self.fresh);
    }

    /// One block of [`BLOCK_FRAMES`] samples in -1..1, filtered in place.
    /// The output is 30 ms behind the input (10 ms more than RNNoise).
    pub fn process(&mut self, samples: &mut [f32]) -> Result<(), String> {
        if samples.len() != BLOCK_FRAMES {
            return Err(format!("{} samples, not {BLOCK_FRAMES}", samples.len()));
        }
        for sample in samples.iter_mut() {
            *sample = sample.clamp(-PEAK, PEAK);
        }
        let noisy = ArrayView2::from_shape((1, BLOCK_FRAMES), &*samples)
            .map_err(|error| error.to_string())?;
        let enhanced = ArrayViewMut2::from_shape((1, BLOCK_FRAMES), &mut self.enhanced[..])
            .map_err(|error| error.to_string())?;
        self.model
            .process(noisy, enhanced)
            .map_err(|error| format!("{error:#}"))?;
        samples.copy_from_slice(&self.enhanced);
        Ok(())
    }
}

/// Hands the functions to the core once; engines created afterwards offer
/// "strong".
pub(crate) fn register() {
    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        static FILTER: ffi::gm_strong_filter = ffi::gm_strong_filter {
            create: Some(create),
            reset: Some(reset),
            process: Some(process),
            destroy: Some(destroy),
        };
        // SAFETY: the core copies the four functions, which live as long
        // as the program and do not unwind.
        unsafe { ffi::gm_set_strong_filter(&FILTER) }
    });
}

/// Copies `message` into the core's buffer, cut to fit with its NUL.
fn write_error(message: &str, error: *mut c_char, size: usize) {
    if error.is_null() || size == 0 {
        return;
    }
    let mut end = message.len().min(size - 1);
    while !message.is_char_boundary(end) {
        end -= 1;
    }
    // SAFETY: the core passes a buffer of `size` bytes; `end < size`.
    unsafe {
        std::ptr::copy_nonoverlapping(message.as_ptr().cast::<c_char>(), error, end);
        *error.add(end) = 0;
    }
}

unsafe extern "C" fn create(error: *mut c_char, size: usize) -> *mut c_void {
    let built = catch_unwind(StrongFilter::new)
        .unwrap_or_else(|_| Err("building DeepFilterNet3 panicked".to_owned()));
    match built {
        Ok(filter) => Box::into_raw(Box::new(filter)).cast(),
        Err(message) => {
            write_error(&message, error, size);
            std::ptr::null_mut()
        }
    }
}

unsafe extern "C" fn reset(filter: *mut c_void) {
    // SAFETY: a filter from `create`, used by one thread at a time.
    let filter = unsafe { &mut *filter.cast::<StrongFilter>() };
    // A panic leaves the filter as it was; the next block may fail.
    let _ = catch_unwind(AssertUnwindSafe(|| filter.reset()));
}

unsafe extern "C" fn process(filter: *mut c_void, samples: *mut f32, count: usize) -> c_int {
    if filter.is_null() || samples.is_null() {
        return -1;
    }
    // SAFETY: a filter from `create`, used by one thread at a time, and
    // `count` samples that the core owns for the call.
    let (filter, samples) = unsafe {
        (
            &mut *filter.cast::<StrongFilter>(),
            std::slice::from_raw_parts_mut(samples, count),
        )
    };
    match catch_unwind(AssertUnwindSafe(|| filter.process(samples))) {
        Ok(Ok(())) => 0,
        _ => -1,
    }
}

unsafe extern "C" fn destroy(filter: *mut c_void) {
    if filter.is_null() {
        return;
    }
    // SAFETY: a filter from `create`, given back once.
    let filter = unsafe { Box::from_raw(filter.cast::<StrongFilter>()) };
    let _ = catch_unwind(AssertUnwindSafe(move || drop(filter)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The core builds the filter on one thread and runs it on another:
    /// the result is the same as on one thread.
    #[test]
    fn a_filter_built_on_one_thread_runs_on_another() {
        let input = noise(50, 0.1);
        let mut here = StrongFilter::new().unwrap();
        let expected = run(&mut here, &input);
        let built = std::thread::spawn(|| StrongFilter::new().unwrap())
            .join()
            .unwrap();
        let moved = input.clone();
        let output = std::thread::spawn(move || {
            let mut filter = built;
            run(&mut filter, &moved)
        })
        .join()
        .unwrap();
        assert_eq!(output, expected);
    }

    fn noise(blocks: usize, amplitude: f32) -> Vec<f32> {
        let mut state = 7u32;
        (0..blocks * BLOCK_FRAMES)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amplitude * (2.0 * (state as f32 / u32::MAX as f32) - 1.0)
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn run(filter: &mut StrongFilter, input: &[f32]) -> Vec<f32> {
        let mut output = input.to_vec();
        for block in output.chunks_mut(BLOCK_FRAMES) {
            filter.process(block).unwrap();
        }
        output
    }

    /// The measured model takes steady white noise out (no speech in it),
    /// and a new stream starts from the same state as a new filter.
    #[test]
    fn deepfilternet3_takes_white_noise_out_and_restarts_cleanly() {
        let mut filter = StrongFilter::new().unwrap();
        assert_eq!((filter.model.sr, filter.model.hop_size), (48_000, 480));
        // 3 blocks of look-ahead and STFT: 30 ms, as measured.
        let delay = filter.model.fft_size - filter.model.hop_size
            + filter.model.lookahead * filter.model.hop_size;
        assert_eq!(delay, 1440);
        let input = noise(200, 0.1);
        let first = run(&mut filter, &input);
        let tail = 100 * BLOCK_FRAMES..;
        assert!(
            rms(&first[tail.clone()]) * 10.0 < rms(&input[tail.clone()]),
            "noise {} -> {}",
            rms(&input[tail.clone()]),
            rms(&first[tail.clone()])
        );
        filter.reset();
        let again = run(&mut filter, &input);
        assert_eq!(first, again, "a reset filter behaves like a new one");
    }

    #[test]
    fn a_block_of_another_size_is_refused() {
        let mut filter = StrongFilter::new().unwrap();
        assert!(filter.process(&mut [0.0; 160]).is_err());
    }

    /// Through the C functions, as the core calls them.
    #[test]
    fn the_c_functions_build_run_and_free_a_filter() {
        let mut error = [0 as c_char; 64];
        // SAFETY: a 64-byte buffer; the filter is used and freed once.
        unsafe {
            let made = create(error.as_mut_ptr(), error.len());
            assert!(!made.is_null());
            let mut block = noise(1, 0.5);
            assert_eq!(process(made, block.as_mut_ptr(), block.len()), 0);
            assert_eq!(process(made, block.as_mut_ptr(), 100), -1);
            reset(made);
            destroy(made);
        }
    }

    #[test]
    fn an_error_is_cut_to_the_cores_buffer() {
        let mut error = [1 as c_char; 6];
        write_error("Rauschfilter", error.as_mut_ptr(), error.len());
        let text: Vec<u8> = error.iter().map(|&c| c as u8).collect();
        assert_eq!(&text, b"Rausc\0");
        let mut error = [1 as c_char; 3];
        write_error("ä", error.as_mut_ptr(), 2);
        assert_eq!(error[0], 0, "a character is not split");
    }
}
