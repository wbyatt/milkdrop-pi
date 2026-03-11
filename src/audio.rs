use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::*, HeapRb};

pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Keeps the audio capture stream alive. Drop to stop capture.
pub struct AudioCapture {
    host: cpal::Host,
    _stream: cpal::Stream,
    device_name: String,
}

/// Lock-free consumer for audio samples from the capture thread.
pub struct AudioReceiver {
    consumer: ringbuf::HeapCons<f32>,
}

impl AudioReceiver {
    pub fn drain(&mut self, buffer: &mut Vec<f32>) {
        buffer.extend(self.consumer.pop_iter());
    }
}

impl AudioCapture {
    /// Starts WASAPI loopback capture on the default output device.
    /// Returns the capture handle, a sample receiver, and the device's audio config.
    pub fn start() -> (Self, AudioReceiver, AudioConfig) {
        let host = cpal::default_host();
        let (stream, receiver, config, device_name) = open_default_device(&host);

        (
            Self {
                host,
                _stream: stream,
                device_name,
            },
            receiver,
            config,
        )
    }

    /// Check if the default output device has changed.
    /// If so, reconnect and return the new receiver and config.
    /// The caller must rebuild the SpectrumAnalyzer if the sample rate changed.
    pub fn reconnect_if_changed(&mut self) -> Option<(AudioReceiver, AudioConfig)> {
        let current = match self.host.default_output_device() {
            Some(d) => d,
            None => return None,
        };
        let current_name = current.name().unwrap_or_default();
        if current_name == self.device_name {
            return None;
        }

        log::info!("audio device changed: {} -> {}", self.device_name, current_name);
        let (stream, receiver, config, device_name) = open_default_device(&self.host);
        self._stream = stream;
        self.device_name = device_name;
        Some((receiver, config))
    }
}

fn open_default_device(host: &cpal::Host) -> (cpal::Stream, AudioReceiver, AudioConfig, String) {
    let device = host
        .default_output_device()
        .expect("no output device available");
    let device_name = device.name().unwrap_or_else(|_| "unknown".to_string());
    let supported_config = device
        .default_output_config()
        .expect("no default output config");

    let sample_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels();

    log::info!("audio device: {} ({}Hz, {} ch)", device_name, sample_rate, channels);

    let capacity = sample_rate as usize * channels as usize;
    let rb = HeapRb::<f32>::new(capacity);
    let (producer, consumer) = rb.split();

    let stream = build_input_stream(&device, &supported_config, producer);
    stream.play().expect("failed to start audio capture");

    (
        stream,
        AudioReceiver { consumer },
        AudioConfig { sample_rate, channels },
        device_name,
    )
}

fn build_input_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    mut producer: ringbuf::HeapProd<f32>,
) -> cpal::Stream {
    let stream_config: cpal::StreamConfig = config.clone().into();

    device
        .build_input_stream(
            &stream_config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let _ = producer.push_slice(data);
            },
            |err| log::error!("audio capture error: {}", err),
            None,
        )
        .expect("failed to build input stream")
}
