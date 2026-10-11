//! Sound out: the score engine on the default output device, or (no device, or
//! `HIM_AUDIO=headless`) rendered in real time into nothing, so the clock still runs.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use second_person::music::{Compiled, Engine};

pub struct Output {
    pub engine: Arc<Mutex<Engine>>,
    pub device: String,
    _stream: Option<cpal::Stream>,
}

pub fn start(score: Compiled) -> Output {
    let headless = std::env::var("HIM_AUDIO").is_ok_and(|v| v == "headless");
    if !headless {
        match open(score.clone()) {
            Ok(out) => return out,
            Err(e) => eprintln!("editor: no sound ({e}); running silent"),
        }
    }
    silent(score)
}

fn open(score: Compiled) -> Result<Output, String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no output device")?;
    let name = device.description().map(|d| d.to_string()).unwrap_or_else(|_| "default".into());
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let format = config.sample_format();
    let config: cpal::StreamConfig = config.into();
    let engine = Arc::new(Mutex::new(Engine::new(score, config.sample_rate as f64)));
    let stream = match format {
        cpal::SampleFormat::F32 => build::<f32>(&device, config, engine.clone()),
        cpal::SampleFormat::I16 => build::<i16>(&device, config, engine.clone()),
        cpal::SampleFormat::I32 => build::<i32>(&device, config, engine.clone()),
        f => return Err(format!("unsupported sample format {f:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(Output { engine, device: name, _stream: Some(stream) })
}

fn build<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    engine: Arc<Mutex<Engine>>,
) -> Result<cpal::Stream, String> {
    let channels = config.channels as usize;
    let mut buf: Vec<f32> = Vec::new();
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                buf.resize(data.len() / channels * 2, 0.0);
                engine.lock().unwrap().render(&mut buf);
                for (i, frame) in data.chunks_mut(channels).enumerate() {
                    for (c, s) in frame.iter_mut().enumerate() {
                        *s = T::from_sample(buf[i * 2 + c.min(1)]);
                    }
                }
            },
            |e: cpal::Error| eprintln!("editor: audio: {e}"),
            None,
        )
        .map_err(|e| e.to_string())
}

fn silent(score: Compiled) -> Output {
    const SR: f64 = 48000.0;
    let engine = Arc::new(Mutex::new(Engine::new(score, SR)));
    let e = engine.clone();
    std::thread::spawn(move || {
        let start = Instant::now();
        let mut buf = vec![0.0f32; 1024];
        loop {
            // Keep the engine's clock level with the wall clock.
            let behind = start.elapsed().as_secs_f64() - e.lock().unwrap().time();
            if behind > 0.0 {
                e.lock().unwrap().render(&mut buf);
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    });
    Output { engine, device: "silent".into(), _stream: None }
}
