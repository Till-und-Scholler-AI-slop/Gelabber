//! DeepFilterNet3 behind a minimal C ABI for wasm32-unknown-unknown, for the
//! Strong noise filter (GEL-28): one 10 ms hop (480 samples at 48 kHz) per
//! call. The module imports nothing.
//!
//! The web client builds the model once in a worker (`df_create`, about half
//! a second), copies the module's memory and hands that copy to the
//! AudioWorklet, which restores it into a fresh instance of the same module
//! and only calls `df_process`. That works because all state lives in linear
//! memory: the module's only mutable global is the stack pointer, which is
//! back at its start whenever an export returns. Keep it that way: no
//! threads, no thread-local storage, no imports.

use df::tract::{DfParams, DfTract, RuntimeParams};
use ndarray::ArrayView2;
use ndarray::ArrayViewMut2;

// Only hash-map seeds ask for randomness. A fixed seed also makes the built
// model, and so the memory copy, the same on every run.
fn fixed_random(buf: &mut [u8]) -> Result<(), getrandom::Error> {
    buf.iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b = i as u8 ^ 0x5a);
    Ok(())
}
getrandom::register_custom_getrandom!(fixed_random);

pub struct State {
    model: DfTract,
    input: Vec<f32>,
    output: Vec<f32>,
}

/// Allocates `len` bytes for the caller to copy the model archive into.
#[no_mangle]
pub extern "C" fn df_alloc(len: usize) -> *mut u8 {
    let mut v = Vec::<u8>::with_capacity(len);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Builds the model from the DeepFilterNet3_onnx.tar.gz bytes at `ptr`, with
/// the attenuation limit and the local-SNR thresholds given. Frees the bytes.
/// Returns null on failure.
///
/// # Safety
/// `ptr` must come from `df_alloc(len)` and hold `len` initialised bytes.
#[no_mangle]
pub unsafe extern "C" fn df_create(
    ptr: *mut u8,
    len: usize,
    atten_lim_db: f32,
    min_db: f32,
    max_erb_db: f32,
    max_df_db: f32,
) -> *mut State {
    let bytes = Vec::from_raw_parts(ptr, len, len);
    let params = match DfParams::from_bytes(&bytes) {
        Ok(p) => p,
        Err(_) => return std::ptr::null_mut(),
    };
    drop(bytes);
    let rp = RuntimeParams::default()
        .with_atten_lim(atten_lim_db)
        .with_thresholds(min_db, max_erb_db, max_df_db);
    match DfTract::new(params, &rp) {
        Ok(model) => {
            let hop = model.hop_size;
            Box::into_raw(Box::new(State {
                model,
                input: vec![0.; hop],
                output: vec![0.; hop],
            }))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// `s` must come from `df_create`.
#[no_mangle]
pub unsafe extern "C" fn df_hop_size(s: *mut State) -> usize {
    (*s).model.hop_size
}

/// # Safety
/// `s` must come from `df_create`.
#[no_mangle]
pub unsafe extern "C" fn df_input(s: *mut State) -> *mut f32 {
    (*s).input.as_mut_ptr()
}

/// # Safety
/// `s` must come from `df_create`.
#[no_mangle]
pub unsafe extern "C" fn df_output(s: *mut State) -> *mut f32 {
    (*s).output.as_mut_ptr()
}

/// Processes the hop in the input buffer into the output buffer and returns
/// the local SNR estimate, or NaN on error.
///
/// # Safety
/// `s` must come from `df_create`.
#[no_mangle]
pub unsafe extern "C" fn df_process(s: *mut State) -> f32 {
    let s = &mut *s;
    let hop = s.model.hop_size;
    let input = ArrayView2::from_shape((1, hop), &s.input[..]).unwrap();
    let output = ArrayViewMut2::from_shape((1, hop), &mut s.output[..]).unwrap();
    s.model.process(input, output).unwrap_or(f32::NAN)
}
