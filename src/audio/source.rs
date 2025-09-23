use jack::Error;

use common::command::{CommandError, ControlCommand};

use common::status::{AudioSourceState, BeatState, TransportState};
use std::fmt::Debug;
use std::ops::Div;

use crate::cbnet::CrossbeamNetwork;

pub struct AudioSourceContext {
    pub jack_time: u64,
    pub frame_size: usize,
    pub sample_rate: usize,
    pub beat: BeatState,
    pub transport: TransportState,
    pub cbnet: CrossbeamNetwork,
}

impl AudioSourceContext {
    pub fn samples_to_next_beat(&self) -> usize {
        (self.transport.us_to_next_beat / 10) * (self.sample_rate / 100) / 1000
    }

    pub fn will_overrun_frame(&self) -> bool {
        self.samples_to_next_beat() < self.frame_size
    }
}

impl Default for AudioSourceContext {
    fn default() -> Self {
        Self {
            jack_time: 0,
            frame_size: 0,
            sample_rate: 0,
            beat: BeatState::default(),
            transport: TransportState::default(),
            cbnet: CrossbeamNetwork::new(),
        }
    }
}

pub trait AudioSource: Send {
    fn send_buffer(&mut self, ctx: &AudioSourceContext) -> Result<&[f32], Error>;
    fn command(
        &mut self,
        ctx: &AudioSourceContext,
        command: ControlCommand,
    ) -> Result<(), CommandError>;
    fn get_status(&mut self, ctx: &AudioSourceContext) -> AudioSourceState;

    fn silence(&self, length: usize) -> &[f32] {
        &[0f32; 2048][0..length]
    }
}

pub struct SourceConfig {
    pub name: String,
    pub source_device: Box<dyn AudioSource>,
    gain_mult: f32,
    gain: f32,
}

impl Debug for SourceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "SourceConfig '{}'", self.name)
    }
}

impl SourceConfig {
    pub fn new(name: String, device: Box<dyn AudioSource>) -> Self {
        Self {
            name,
            source_device: device,
            gain_mult: 1.0,
            gain: 0.0,
        }
    }
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain;
        self.gain_mult = 10.0f32.powf(gain.div(20.0))
    }

    pub fn get_gain_mult(&self) -> f32 {
        self.gain_mult
    }
    pub fn get_gain(&self) -> f32 {
        self.gain
    }
}
