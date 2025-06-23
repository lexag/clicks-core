use jack::{Client, Error, ProcessScope};

use common::command::{CommandError, ControlCommand};

use common::status::{AudioSourceStatus, ProcessStatus};

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
}
