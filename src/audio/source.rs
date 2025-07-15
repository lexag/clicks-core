use jack::{Client, Error, ProcessScope};

use common::command::{CommandError, ControlCommand};

use common::status::{AudioSourceStatus, ProcessStatus};
use std::fmt::Debug;
use std::ops::Div;

pub trait AudioSource: Send {
    fn send_buffer(
        &mut self,
        _c: &Client,
        _ps: &ProcessScope,
        status: ProcessStatus,
    ) -> Result<&[f32], Error>;
    fn command(&mut self, command: ControlCommand) -> Result<(), CommandError>;
    fn get_status(&mut self, _c: &Client, _ps: &ProcessScope) -> AudioSourceStatus;
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
