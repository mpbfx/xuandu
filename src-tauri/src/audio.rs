use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
        Arc,
    },
    thread,
    time::Duration,
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;

const MAX_BUFFER_SECONDS: u32 = 12;
const WRITE_BATCH_SAMPLES: usize = 2_048;
const FULL_BUFFER_WAIT: Duration = Duration::from_millis(4);
const PROVIDER_SAMPLE_RATES: &[u32] = &[8_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000];

/// A bounded PCM queue shared by the network task and the audio callback.
///
/// The producer waits when the queue is full instead of dropping samples. That
/// backpressures the HTTP stream, preserving the complete speech sequence for
/// long selections while keeping memory use bounded.
struct SharedQueue {
    state: Mutex<QueueState>,
    maximum_samples: usize,
}

struct QueueState {
    samples: VecDeque<i16>,
    remainder: Option<u8>,
}

impl SharedQueue {
    fn new(maximum_samples: usize) -> Self {
        Self {
            state: Mutex::new(QueueState {
                samples: VecDeque::new(),
                remainder: None,
            }),
            maximum_samples,
        }
    }

    async fn append_pcm(&self, cancellation: &CancellationToken, chunk: &[u8]) {
        let mut offset = 0;
        while offset < chunk.len() {
            if cancellation.is_cancelled() {
                return;
            }

            let advanced = {
                let mut state = self.state.lock();
                append_available_samples(&mut state, self.maximum_samples, chunk, &mut offset)
            };
            if offset == chunk.len() {
                return;
            }

            if advanced {
                // Large HTTP chunks must not monopolize a Tokio worker while
                // the real-time callback is consuming the same queue.
                tokio::task::yield_now().await;
            } else {
                tokio::select! {
                    _ = cancellation.cancelled() => return,
                    _ = tokio::time::sleep(FULL_BUFFER_WAIT) => {}
                }
            }
        }
    }
}

fn append_available_samples(
    state: &mut QueueState,
    maximum_samples: usize,
    chunk: &[u8],
    offset: &mut usize,
) -> bool {
    let initial_offset = *offset;
    let mut available = maximum_samples
        .saturating_sub(state.samples.len())
        .min(WRITE_BATCH_SAMPLES);
    if available == 0 {
        return false;
    }

    if let Some(first) = state.remainder.take() {
        if *offset == chunk.len() {
            state.remainder = Some(first);
            return false;
        }
        state
            .samples
            .push_back(i16::from_le_bytes([first, chunk[*offset]]));
        *offset += 1;
        available -= 1;
    }

    while available > 0 && *offset + 1 < chunk.len() {
        state
            .samples
            .push_back(i16::from_le_bytes([chunk[*offset], chunk[*offset + 1]]));
        *offset += 2;
        available -= 1;
    }

    if *offset + 1 == chunk.len() {
        state.remainder = Some(chunk[*offset]);
        *offset += 1;
    }

    *offset > initial_offset
}

/// Tauri state is Send + Sync, whereas a CoreAudio stream is intentionally not.
/// The stream therefore lives on this dedicated worker thread and receives only
/// lifecycle commands from the application state.
pub struct PlaybackController {
    commands: Sender<AudioCommand>,
    next_session: AtomicU64,
    active: Mutex<Option<ActiveSession>>,
}

#[derive(Clone)]
struct ActiveSession {
    id: u64,
    cancellation: CancellationToken,
}

struct StartedSession {
    sample_rate: u32,
    queue: Arc<SharedQueue>,
}

enum AudioCommand {
    Start {
        id: u64,
        response: SyncSender<Result<StartedSession, String>>,
    },
    Stop {
        id: u64,
    },
}

#[derive(Clone)]
pub struct PlaybackHandle {
    cancellation: CancellationToken,
    sample_rate: u32,
    queue: Arc<SharedQueue>,
}

impl Default for PlaybackController {
    fn default() -> Self {
        let (commands, receiver) = mpsc::channel();
        thread::spawn(move || audio_worker(receiver));
        Self {
            commands,
            next_session: AtomicU64::new(0),
            active: Mutex::new(None),
        }
    }
}

impl PlaybackController {
    pub fn start(&self) -> Result<PlaybackHandle, String> {
        self.stop();
        let id = self.next_session.fetch_add(1, Ordering::SeqCst) + 1;
        let cancellation = CancellationToken::new();
        let (response_tx, response_rx) = mpsc::sync_channel(1);

        self.commands
            .send(AudioCommand::Start {
                id,
                response: response_tx,
            })
            .map_err(|_| "音频工作线程不可用。".to_owned())?;
        let StartedSession { sample_rate, queue } = response_rx
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "启动音频输出超时。".to_owned())??;

        *self.active.lock() = Some(ActiveSession {
            id,
            cancellation: cancellation.clone(),
        });
        Ok(PlaybackHandle {
            cancellation,
            sample_rate,
            queue,
        })
    }

    pub fn stop(&self) {
        if let Some(active) = self.active.lock().take() {
            active.cancellation.cancel();
            let _ = self.commands.send(AudioCommand::Stop { id: active.id });
        }
    }
}

impl PlaybackHandle {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub async fn push_pcm(&self, chunk: &[u8]) {
        if !self.cancellation.is_cancelled() && !chunk.is_empty() {
            self.queue.append_pcm(&self.cancellation, chunk).await;
        }
    }

    pub async fn cancelled(&self) {
        self.cancellation.cancelled().await;
    }
}

fn audio_worker(receiver: Receiver<AudioCommand>) {
    let mut session: Option<WorkerSession> = None;
    while let Ok(command) = receiver.recv() {
        match command {
            AudioCommand::Start { id, response } => {
                session = None;
                let result = WorkerSession::new(id);
                match result {
                    Ok((next, started)) => {
                        session = Some(next);
                        let _ = response.send(Ok(started));
                    }
                    Err(error) => {
                        let _ = response.send(Err(error));
                    }
                }
            }
            AudioCommand::Stop { id } => {
                if session.as_ref().is_some_and(|current| current.id == id) {
                    session = None;
                }
            }
        }
    }
}

struct WorkerSession {
    id: u64,
    _stream: Stream,
}

impl WorkerSession {
    fn new(id: u64) -> Result<(Self, StartedSession), String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "没有可用的音频输出设备。".to_owned())?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("无法读取音频设备：{error}"))?;
        let sample_rate = supported.sample_rate().0;

        if !PROVIDER_SAMPLE_RATES.contains(&sample_rate) {
            return Err(format!(
                "当前输出设备使用 {sample_rate} Hz，豆包语音暂不支持该采样率。"
            ));
        }

        let queue = Arc::new(SharedQueue::new(
            sample_rate as usize * MAX_BUFFER_SECONDS as usize,
        ));
        let config: StreamConfig = supported.clone().into();
        let stream = build_stream(
            &device,
            &config,
            supported.sample_format(),
            Arc::clone(&queue),
            config.channels as usize,
        )?;
        stream
            .play()
            .map_err(|error| format!("无法开始音频播放：{error}"))?;

        Ok((
            Self {
                id,
                _stream: stream,
            },
            StartedSession { sample_rate, queue },
        ))
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    queue: Arc<SharedQueue>,
    channels: usize,
) -> Result<Stream, String> {
    let error_handler = |error| eprintln!("audio output error: {error}");
    match sample_format {
        SampleFormat::I16 => device
            .build_output_stream(
                config,
                move |data: &mut [i16], _| fill_i16(data, channels, &queue),
                error_handler,
                None,
            )
            .map_err(|error| format!("无法创建音频流：{error}")),
        SampleFormat::F32 => device
            .build_output_stream(
                config,
                move |data: &mut [f32], _| fill_f32(data, channels, &queue),
                error_handler,
                None,
            )
            .map_err(|error| format!("无法创建音频流：{error}")),
        SampleFormat::U16 => device
            .build_output_stream(
                config,
                move |data: &mut [u16], _| fill_u16(data, channels, &queue),
                error_handler,
                None,
            )
            .map_err(|error| format!("无法创建音频流：{error}")),
        format => Err(format!("不支持的本机音频格式：{format:?}")),
    }
}

fn fill_i16(data: &mut [i16], channels: usize, queue: &SharedQueue) {
    let mut state = queue.state.lock();
    for frame in data.chunks_mut(channels) {
        frame.fill(state.samples.pop_front().unwrap_or(0));
    }
}

fn fill_f32(data: &mut [f32], channels: usize, queue: &SharedQueue) {
    let mut state = queue.state.lock();
    for frame in data.chunks_mut(channels) {
        let source = state.samples.pop_front().unwrap_or(0) as f32 / i16::MAX as f32;
        frame.fill(source);
    }
}

fn fill_u16(data: &mut [u16], channels: usize, queue: &SharedQueue) {
    let mut state = queue.state.lock();
    for frame in data.chunks_mut(channels) {
        frame.fill((state.samples.pop_front().unwrap_or(0) as i32 + 32_768) as u16);
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use tokio_util::sync::CancellationToken;

    use super::SharedQueue;

    #[tokio::test]
    async fn queue_waits_for_capacity_instead_of_discarding_later_samples() {
        let queue = Arc::new(SharedQueue::new(1));
        let cancellation = CancellationToken::new();
        let producer_queue = Arc::clone(&queue);
        let producer_cancellation = cancellation.clone();
        let producer = tokio::spawn(async move {
            producer_queue
                .append_pcm(&producer_cancellation, &[1, 0, 2, 0])
                .await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(queue.state.lock().samples.pop_front(), Some(1));

        producer.await.unwrap();
        assert_eq!(queue.state.lock().samples.pop_front(), Some(2));
    }
}
