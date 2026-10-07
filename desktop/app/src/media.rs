//! Tauri commands over the native media core, for the server's web client.
//!
//! The web client keeps all signaling (media WebSocket, tickets, ACL, source
//! epochs) and drives these commands the way it drives `mediasoup-client`:
//! load a device, create transports, answer their `connect`/`produce`
//! requests through the server, produce local sources, consume announced
//! producers. Objects are handles (numbers) into a per-app registry; a page
//! load drops every object the previous page left behind.
use crate::viewer::Viewer;
use gelabber_media_core::{
    Audio, Consumer, Device, Direction, Engine, MediaKind, Producer, Source, Transport,
    TransportEvent,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tauri::{State, async_runtime::spawn_blocking, ipc::Channel};

#[cfg(test)]
include!("commands.rs");

type Shared<T> = Arc<Mutex<T>>;

#[derive(Default)]
pub struct Media {
    engine: OnceLock<Engine>,
    next: AtomicU64,
    devices: Mutex<HashMap<u64, Device>>,
    transports: Mutex<HashMap<u64, Transport>>,
    sources: Mutex<HashMap<u64, Source>>,
    producers: Mutex<HashMap<u64, Shared<Producer>>>,
    consumers: Mutex<HashMap<u64, Shared<Consumer>>>,
}

type Result<T> = std::result::Result<T, String>;

fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn get<T: Clone>(map: &Mutex<HashMap<u64, T>>, handle: u64, what: &str) -> Result<T> {
    map.lock()
        .unwrap()
        .get(&handle)
        .cloned()
        .ok_or_else(|| format!("unknown {what} {handle}"))
}

impl Media {
    fn engine(&self) -> Result<&Engine> {
        if let Some(engine) = self.engine.get() {
            return Ok(engine);
        }
        let engine = Engine::new(Audio::Default).map_err(err)?;
        // A racing first call loses its engine; both are equivalent.
        Ok(self.engine.get_or_init(|| engine))
    }

    fn insert<T>(&self, map: &Mutex<HashMap<u64, T>>, value: T) -> u64 {
        let handle = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        map.lock().unwrap().insert(handle, value);
        handle
    }

    /// Drops every object of a page that went away. Producers and consumers
    /// go first: they close on their transports.
    pub fn reset(&self) {
        self.consumers.lock().unwrap().clear();
        self.producers.lock().unwrap().clear();
        self.sources.lock().unwrap().clear();
        self.transports.lock().unwrap().clear();
        self.devices.lock().unwrap().clear();
        if let Some(viewer) = Viewer::running() {
            viewer.close_all();
        }
        if let Some(engine) = self.engine.get() {
            // A microphone test the page left open.
            let _ = engine.monitor_audio(None);
        }
    }
}

/// Runs a native call that may block (produce/consume wait for the server
/// round trips the page answers) off the async runtime.
async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T> {
    spawn_blocking(job).await.map_err(err)?
}

#[tauri::command]
pub async fn media_info() -> Value {
    json!({
        "abi": gelabber_media_core::ABI_VERSION,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
    })
}

#[tauri::command]
pub async fn media_audio_devices(media: State<'_, Media>) -> Result<Value> {
    media.engine()?.audio_devices().map_err(err)
}

#[tauri::command]
pub async fn media_audio_configure(media: State<'_, Media>, options: Value) -> Result<()> {
    media.engine()?.configure_audio(&options).map_err(err)
}

#[tauri::command]
pub async fn media_audio_levels(media: State<'_, Media>) -> Result<Value> {
    media.engine()?.audio_levels().map_err(err)
}

/// The microphone test: `options` (`{"processingMode","inputGain"}`) starts
/// it, `null` ends it.
#[tauri::command]
pub async fn media_audio_monitor(media: State<'_, Media>, options: Option<Value>) -> Result<()> {
    let engine = media.engine()?.clone();
    blocking(move || engine.monitor_audio(options.as_ref()).map_err(err)).await
}

#[tauri::command]
pub async fn media_device_load(media: State<'_, Media>, capabilities: Value) -> Result<Value> {
    let device = Device::new(media.engine()?).map_err(err)?;
    device.load(&capabilities).map_err(err)?;
    let mut result = json!({
        "rtpCapabilities": device.rtp_capabilities().map_err(err)?,
        "canProduce": {
            "audio": device.can_produce(MediaKind::Audio).map_err(err)?,
            "video": device.can_produce(MediaKind::Video).map_err(err)?,
        },
    });
    result["device"] = media.insert(&media.devices, device).into();
    Ok(result)
}

#[tauri::command]
pub async fn media_device_close(media: State<'_, Media>, device: u64) -> Result<()> {
    media.devices.lock().unwrap().remove(&device);
    Ok(())
}

/// `events` receives `{"type":"connect","request","dtlsParameters"}`,
/// `{"type":"produce","request","kind","rtpParameters","appData"}` and
/// `{"type":"connectionstatechange","state"}`; answer the first two with
/// `media_transport_respond`.
#[tauri::command]
pub async fn media_transport_create(
    media: State<'_, Media>,
    device: u64,
    direction: String,
    options: Value,
    events: Channel<Value>,
) -> Result<Value> {
    let device = get(&media.devices, device, "device")?;
    let direction = match direction.as_str() {
        "send" => Direction::Send,
        "recv" => Direction::Recv,
        other => return Err(format!("unknown direction {other}")),
    };
    let (transport, receiver) = Transport::new(&device, direction, &options).map_err(err)?;
    let id = transport.id().to_owned();
    std::thread::Builder::new()
        .name("gelabber-transport-events".into())
        .spawn(move || {
            // Ends when the transport is freed and drops its sender.
            for event in receiver {
                let message = match event {
                    TransportEvent::Connect {
                        request,
                        dtls_parameters,
                    } => json!({
                        "type": "connect",
                        "request": request,
                        "dtlsParameters": dtls_parameters,
                    }),
                    TransportEvent::Produce {
                        request,
                        kind,
                        rtp_parameters,
                        app_data,
                    } => json!({
                        "type": "produce",
                        "request": request,
                        "kind": kind,
                        "rtpParameters": rtp_parameters,
                        "appData": app_data,
                    }),
                    TransportEvent::ConnectionState(state) => json!({
                        "type": "connectionstatechange",
                        "state": state,
                    }),
                };
                if events.send(message).is_err() {
                    break;
                }
            }
        })
        .map_err(err)?;
    let handle = media.insert(&media.transports, transport);
    Ok(json!({ "transport": handle, "id": id }))
}

/// Exactly one of `result`/`error`.
#[tauri::command]
pub async fn media_transport_respond(
    media: State<'_, Media>,
    transport: u64,
    request: u64,
    result: Option<Value>,
    error: Option<String>,
) -> Result<()> {
    let transport = get(&media.transports, transport, "transport")?;
    let answer = match (result, error) {
        (Some(value), None) => Ok(value),
        (None, Some(message)) => Err(message),
        _ => return Err("respond with either result or error".into()),
    };
    transport.respond(request, answer).map_err(err)
}

#[tauri::command]
pub async fn media_transport_restart_ice(
    media: State<'_, Media>,
    transport: u64,
    ice_parameters: Value,
) -> Result<()> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.restart_ice(&ice_parameters).map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_stats(media: State<'_, Media>, transport: u64) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    blocking(move || transport.stats().map_err(err)).await
}

#[tauri::command]
pub async fn media_transport_close(media: State<'_, Media>, transport: u64) -> Result<()> {
    let removed = media.transports.lock().unwrap().remove(&transport);
    // Producers and consumers keep the transport alive until they close.
    drop(removed);
    Ok(())
}

#[tauri::command]
pub async fn media_source_microphone(media: State<'_, Media>, options: Value) -> Result<u64> {
    let source = Source::microphone(media.engine()?, &options).map_err(err)?;
    Ok(media.insert(&media.sources, source))
}

/// Opens the desktop's own screen/window picker; poll `media_source_state`.
#[tauri::command]
pub async fn media_source_screen(media: State<'_, Media>, options: Value) -> Result<u64> {
    let source = Source::screen(media.engine()?, &options).map_err(err)?;
    Ok(media.insert(&media.sources, source))
}

#[tauri::command]
pub async fn media_source_state(media: State<'_, Media>, source: u64) -> Result<Value> {
    get(&media.sources, source, "source")?.state().map_err(err)
}

#[tauri::command]
pub async fn media_source_set_enabled(
    media: State<'_, Media>,
    source: u64,
    enabled: bool,
) -> Result<()> {
    get(&media.sources, source, "source")?
        .set_enabled(enabled)
        .map_err(err)
}

/// Producers keep their source running until they close too.
#[tauri::command]
pub async fn media_source_close(media: State<'_, Media>, source: u64) -> Result<()> {
    media.sources.lock().unwrap().remove(&source);
    Ok(())
}

#[tauri::command]
pub async fn media_produce(
    media: State<'_, Media>,
    transport: u64,
    source: u64,
    options: Value,
) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    let source = get(&media.sources, source, "source")?;
    let producer = blocking(move || transport.produce(&source, &options).map_err(err)).await?;
    let mut result = json!({
        "id": producer.id(),
        "rtpParameters": producer.rtp_parameters().map_err(err)?,
    });
    result["producer"] = media
        .insert(&media.producers, Arc::new(Mutex::new(producer)))
        .into();
    Ok(result)
}

fn producer(media: &Media, handle: u64) -> Result<Shared<Producer>> {
    get(&media.producers, handle, "producer")
}

#[tauri::command]
pub async fn media_producer_pause(
    media: State<'_, Media>,
    producer: u64,
    paused: bool,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    let producer = producer.lock().unwrap();
    producer.set_paused(paused).map_err(err)
}

#[tauri::command]
pub async fn media_producer_replace_source(
    media: State<'_, Media>,
    producer: u64,
    source: u64,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    let source = get(&media.sources, source, "source")?;
    blocking(move || {
        producer
            .lock()
            .unwrap()
            .replace_source(&source)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn media_producer_parameters(media: State<'_, Media>, producer: u64) -> Result<Value> {
    let producer = self::producer(&media, producer)?;
    let producer = producer.lock().unwrap();
    producer.parameters().map_err(err)
}

#[tauri::command]
pub async fn media_producer_set_parameters(
    media: State<'_, Media>,
    producer: u64,
    parameters: Value,
) -> Result<()> {
    let producer = self::producer(&media, producer)?;
    blocking(move || {
        producer
            .lock()
            .unwrap()
            .set_parameters(&parameters)
            .map_err(err)
    })
    .await
}

#[tauri::command]
pub async fn media_producer_stats(media: State<'_, Media>, producer: u64) -> Result<Value> {
    let producer = self::producer(&media, producer)?;
    blocking(move || producer.lock().unwrap().stats().map_err(err)).await
}

#[tauri::command]
pub async fn media_producer_close(media: State<'_, Media>, producer: u64) -> Result<()> {
    let removed = media.producers.lock().unwrap().remove(&producer);
    if let Some(producer) = removed {
        blocking(move || {
            drop(producer);
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// `params`: the server's consumer announcement as
/// `{"id","producerId","kind","rtpParameters","appData"?}`. Audio starts at
/// volume 0; `media_consumer_set_volume` makes it audible.
#[tauri::command]
pub async fn media_consume(
    media: State<'_, Media>,
    transport: u64,
    params: Value,
) -> Result<Value> {
    let transport = get(&media.transports, transport, "transport")?;
    let audio = params["kind"] == "audio";
    let consumer = blocking(move || {
        let consumer = transport.consume(&params).map_err(err)?;
        // Silent until the page attaches it to an output, like a browser
        // track without an audio element.
        if audio {
            consumer.set_volume(0.0).map_err(err)?;
        }
        Ok(consumer)
    })
    .await?;
    let id = consumer.id().to_owned();
    let handle = media.insert(&media.consumers, Arc::new(Mutex::new(consumer)));
    Ok(json!({ "consumer": handle, "id": id }))
}

fn consumer(media: &Media, handle: u64) -> Result<Shared<Consumer>> {
    get(&media.consumers, handle, "consumer")
}

#[tauri::command]
pub async fn media_consumer_pause(
    media: State<'_, Media>,
    consumer: u64,
    paused: bool,
) -> Result<()> {
    let consumer = self::consumer(&media, consumer)?;
    let consumer = consumer.lock().unwrap();
    consumer.set_paused(paused).map_err(err)
}

/// Audio playback volume, 0..2.
#[tauri::command]
pub async fn media_consumer_set_volume(
    media: State<'_, Media>,
    consumer: u64,
    volume: f64,
) -> Result<()> {
    let consumer = self::consumer(&media, consumer)?;
    let consumer = consumer.lock().unwrap();
    consumer.set_volume(volume).map_err(err)
}

#[tauri::command]
pub async fn media_consumer_stats(media: State<'_, Media>, consumer: u64) -> Result<Value> {
    let consumer = self::consumer(&media, consumer)?;
    blocking(move || consumer.lock().unwrap().stats().map_err(err)).await
}

/// Shows a video consumer in a native viewer window titled `title`.
/// `events` receives `{"type":"closed"}` when the window goes away; the
/// page then calls `media_viewer_close`.
#[tauri::command]
pub async fn media_viewer_open(
    media: State<'_, Media>,
    consumer: u64,
    title: String,
    events: Channel<Value>,
) -> Result<()> {
    let shared = self::consumer(&media, consumer)?;
    let viewer = Viewer::get()?;
    let sink = viewer.open(
        consumer,
        title,
        Box::new(move || {
            let _ = events.send(json!({"type": "closed"}));
        }),
    )?;
    let installed = blocking(move || {
        shared
            .lock()
            .unwrap()
            .set_video_sink(Some(sink))
            .map_err(err)
    })
    .await;
    if installed.is_err() {
        viewer.close(consumer);
    }
    installed
}

#[tauri::command]
pub async fn media_viewer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    if let Some(viewer) = Viewer::running() {
        viewer.close(consumer);
    }
    let Ok(shared) = self::consumer(&media, consumer) else {
        return Ok(());
    };
    blocking(move || shared.lock().unwrap().set_video_sink(None).map_err(err)).await
}

#[tauri::command]
pub async fn media_consumer_close(media: State<'_, Media>, consumer: u64) -> Result<()> {
    if let Some(viewer) = Viewer::running() {
        viewer.close(consumer);
    }
    let removed = media.consumers.lock().unwrap().remove(&consumer);
    if let Some(consumer) = removed {
        blocking(move || {
            drop(consumer);
            Ok(())
        })
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::MEDIA_COMMANDS;

    /// The server origin's permission set grants exactly the media commands.
    #[test]
    fn media_permission_set_matches_the_commands() {
        let set = include_str!("../permissions/media.toml");
        let granted: Vec<&str> = set
            .lines()
            .filter_map(|line| line.trim().strip_prefix("\"allow-"))
            .filter_map(|rest| rest.strip_suffix("\","))
            .collect();
        let expected: Vec<String> = MEDIA_COMMANDS.iter().map(|c| c.replace('_', "-")).collect();
        assert_eq!(granted, expected);
        let main = include_str!("main.rs");
        for command in MEDIA_COMMANDS {
            assert!(
                main.contains(&format!("media::{command},")),
                "{command} missing from generate_handler!"
            );
        }
    }
}
