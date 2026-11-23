//! Runs DeepFilterNet3 (libDF with tract) over 48 kHz mono WAV files one
//! 10 ms hop at a time, as a real-time pipeline would, and times each hop.
//!
//! Usage: dfn-bench <DeepFilterNet3_onnx.tar.gz> <out-dir> <in.wav>...
//! Prints one JSON object to stdout; writes <out-dir>/<name> as 32-bit float
//! WAV without delay compensation (the real-time output).

use std::{fs, io::Write, path::PathBuf, time::Instant};

use anyhow::{bail, Context, Result};
use df::{tract::*, wav_utils::ReadWav};
use ndarray::prelude::*;

fn write_f32_wav(path: &PathBuf, rate: u32, x: &[f32]) -> Result<()> {
    let mut b = Vec::with_capacity(44 + x.len() * 4);
    let data = (x.len() * 4) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for s in x {
        b.extend_from_slice(&s.to_le_bytes());
    }
    fs::File::create(path)?.write_all(&b)?;
    Ok(())
}

/// CPU time of the calling thread in milliseconds.
fn thread_cpu_ms() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid pointer to a timespec; the clock id is a constant.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 * 1e3 + t.tv_nsec as f64 / 1e6
}

/// User + system CPU seconds of this process, to show whether tract used
/// more than one thread (wall time per hop alone would hide that).
fn process_cpu_seconds() -> f64 {
    let stat = fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields: Vec<&str> = stat
        .rsplit(')')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .collect();
    // Fields after the command name: utime is the 12th, stime the 13th.
    let ticks = |i: usize| {
        fields
            .get(i)
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.)
    };
    (ticks(11) + ticks(12)) / 100.
}

fn stats(times: &mut [f64]) -> String {
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = times.len();
    let pct = |p: f64| times[((p / 100. * n as f64) as usize).min(n - 1)];
    let mean = times.iter().sum::<f64>() / n as f64;
    format!(
        "{{\"frames\":{n},\"medianMs\":{:.6},\"p99Ms\":{:.6},\"maxMs\":{:.6},\"meanMs\":{:.6}}}",
        pct(50.),
        pct(99.),
        times[n - 1],
        mean
    )
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        bail!("usage: dfn-bench <model.tar.gz> <out-dir> <in.wav>...");
    }
    let model_path = PathBuf::from(&args[1]);
    let out_dir = PathBuf::from(&args[2]);
    fs::create_dir_all(&out_dir)?;
    // The library defaults are the real-time ones (LADSPA plugin, C API):
    // attenuation limit 100 dB, local-SNR thresholds -10/30/20 dB.
    let mut r_params = RuntimeParams::default();
    // DFN_PARAMS="atten_lim_db,min,max_erb,max_df[,post_filter_beta]" overrides
    // them, e.g. "100,-15,35,35" for the deep-filter command line defaults.
    if let Ok(spec) = std::env::var("DFN_PARAMS") {
        let v: Vec<f32> = spec
            .split(',')
            .map(|s| s.trim().parse())
            .collect::<Result<_, _>>()?;
        if v.len() < 4 {
            bail!("DFN_PARAMS needs atten_lim_db,min,max_erb,max_df[,post_filter_beta]");
        }
        r_params = r_params
            .with_atten_lim(v[0])
            .with_thresholds(v[1], v[2], v[3]);
        if let Some(&beta) = v.get(4) {
            r_params = r_params.with_post_filter(beta);
        }
    }
    let df_params = DfParams::new(model_path.clone()).context("loading model")?;

    let mut all = Vec::new();
    let mut all_cpu = Vec::new();
    let mut files = Vec::new();
    let mut constants = String::new();
    let cpu0 = process_cpu_seconds();
    let mut wall = 0f64;
    let mut stages = [0usize; 5];
    for input in &args[3..] {
        let t_init = Instant::now();
        // A fresh model per file, like a new call.
        let mut model = DfTract::new(df_params.clone(), &r_params)?;
        let init_ms = t_init.elapsed().as_secs_f64() * 1e3;
        if constants.is_empty() {
            constants = format!(
                "\"sampleRate\":{},\"fftSize\":{},\"hopSize\":{},\"lookahead\":{},\"stftPlusLookaheadDelaySamples\":{}",
                model.sr,
                model.fft_size,
                model.hop_size,
                model.lookahead,
                model.fft_size - model.hop_size + model.lookahead * model.hop_size
            );
        }
        let reader = ReadWav::new(input)?;
        if reader.sr != model.sr || reader.channels != 1 {
            bail!("{input}: need {} Hz mono", model.sr);
        }
        let noisy = reader.samples_arr2()?;
        let noisy = noisy.as_standard_layout();
        let hop = model.hop_size;
        let mut enh = Array2::<f32>::zeros(noisy.raw_dim());
        let mut times = Vec::new();
        let mut cpu_times = Vec::new();
        for (ns, en) in noisy
            .axis_chunks_iter(Axis(1), hop)
            .zip(enh.axis_chunks_iter_mut(Axis(1), hop))
        {
            if ns.len_of(Axis(1)) < hop {
                break;
            }
            let c = thread_cpu_ms();
            let t = Instant::now();
            let lsnr = model.process(ns, en)?;
            let ms = t.elapsed().as_secs_f64() * 1e3;
            cpu_times.push(thread_cpu_ms() - c);
            wall += ms / 1e3;
            times.push(ms);
            // Which model stages libDF ran for this hop (DfTract::apply_stages).
            // Its near-silence bypass (mean square below 1e-7 for more than
            // five hops) outputs zeros and returns exactly -15 dB.
            stages[if lsnr == -15. {
                4
            } else {
                match model.apply_stages(lsnr) {
                    (true, _, true) => 0,  // full model
                    (true, _, false) => 1, // ERB gains only, deep filter skipped
                    (false, true, _) => 2, // zero mask, decoders skipped
                    _ => 3,                // passed through unprocessed
                }
            }] += 1;
        }
        all.extend_from_slice(&times);
        all_cpu.extend_from_slice(&cpu_times);
        let name = PathBuf::from(input).file_name().unwrap().to_owned();
        write_f32_wav(
            &out_dir.join(&name),
            model.sr as u32,
            enh.row(0).as_slice().unwrap(),
        )?;
        files.push(format!(
            "{{\"file\":{:?},\"initMs\":{:.3},\"compute\":{},\"threadCpu\":{}}}",
            name.to_string_lossy(),
            init_ms,
            stats(&mut times),
            stats(&mut cpu_times)
        ));
    }
    let cpu = process_cpu_seconds() - cpu0;
    println!(
        "{{\"engine\":\"DeepFilterNet3 (libDF + tract, native)\",{constants},\"runtimeParams\":{{\"attenLimDb\":{},\"minDbThresh\":{},\"maxDbErbThresh\":{},\"maxDbDfThresh\":{},\"postFilter\":{}}},\"compute\":{},\"threadCpu\":{},\"processCpuSeconds\":{cpu:.2},\"hopWallSeconds\":{wall:.2},\"hopStages\":{{\"fullModel\":{},\"erbOnly\":{},\"zeroMask\":{},\"passThrough\":{},\"silenceBypass\":{}}},\"files\":[{}]}}",
        r_params.atten_lim_db,
        r_params.min_db_thresh,
        r_params.max_db_erb_thresh,
        r_params.max_db_df_thresh,
        r_params.post_filter,
        stats(&mut all),
        stats(&mut all_cpu),
        stages[0],
        stages[1],
        stages[2],
        stages[3],
        stages[4],
        files.join(",")
    );
    Ok(())
}
