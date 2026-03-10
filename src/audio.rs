use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{traits::*, HeapRb};

pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Keeps the audio capture stream alive. Drop to stop capture.
pub struct AudioCapture {
    _stream: cpal::Stream,
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
        let device = host
            .default_output_device()
            .expect("no output device available");
        let supported_config = device
            .default_output_config()
            .expect("no default output config");

        let sample_rate = supported_config.sample_rate().0;
        let channels = supported_config.channels();

        log::info!("audio device: {}Hz, {} ch", sample_rate, channels);

        // Ring buffer holds ~1 second of audio
        let capacity = sample_rate as usize * channels as usize;
        let rb = HeapRb::<f32>::new(capacity);
        let (producer, consumer) = rb.split();

        let stream = build_input_stream(&device, &supported_config, producer);
        stream.play().expect("failed to start audio capture");

        (
            Self { _stream: stream },
            AudioReceiver { consumer },
            AudioConfig {
                sample_rate,
                channels,
            },
        )
    }
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
