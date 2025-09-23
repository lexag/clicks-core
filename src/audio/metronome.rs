use common::status::{BeatState, TransportState};

use crate::audio;
use crate::audio::source::AudioSourceContext;
use common::command::{CommandError, ControlCommand};
use common::{
    cue::{BeatEvent, Cue},
    status::AudioSourceState,
};

struct MetronomeClick {
    frequency: usize,
    length: usize,
}

pub struct Metronome {
    clicks: Vec<MetronomeClick>,
    click_buffers: [[f32; 96000]; 2],
    last_beat_time: u64,
    cue: Cue,
    state: BeatState,
    transport: TransportState,
}

impl Default for Metronome {
    fn default() -> Metronome {
        Metronome {
            clicks: vec![
                MetronomeClick {
                    length: 4,
                    frequency: 2000,
                },
                MetronomeClick {
                    length: 4,
                    frequency: 1000,
                },
            ],
            last_beat_time: 0,
            cue: Cue::empty(),
            click_buffers: [[0f32; 96000]; 2],
            state: BeatState::default(),
            transport: TransportState::default(),
        }
    }
}

impl Metronome {
    pub fn new() -> Metronome {
        let mut met = Metronome {
            ..Default::default()
        };
        met.pregen_click_bufs();
        return met;
    }

    pub fn pregen_click_bufs(&mut self) {
        for i in 0..2 {
            let click = &self.clicks[i];
            let mut buf = [0f32; 96000];
            for i in 0..click.length * 48 {
                buf[i] = ((i as f32 * std::f32::consts::PI * click.frequency as f32 / 24000.0)
                    .sin()
                    * 0.1) as f32
            }
            self.click_buffers[i] = buf;
        }
    }
    fn handle_event(&mut self, event: BeatEvent, ctx: &audio::source::AudioSourceContext) {
        match event {
            BeatEvent::JumpEvent {
                destination,
                requirement,
                when_jumped,
                when_passed,
            } => {
                let requirement_fullfilled = match requirement {
                    common::cue::JumpRequirement::JumpModeOn => ctx.transport.vlt,
                    common::cue::JumpRequirement::JumpModeOff => !ctx.transport.vlt,
                    common::cue::JumpRequirement::None => true,
                };

                if requirement_fullfilled {
                    self.state.next_beat_idx = destination;
                    self.state.requested_vlt_action = when_jumped;
                } else {
                    self.state.requested_vlt_action = when_passed;
                }
            }
            _ => {}
        }
    }
}

impl audio::source::AudioSource for Metronome {
    fn get_status(&mut self, ctx: &audio::source::AudioSourceContext) -> AudioSourceState {
        let scheduled_time: u64;
        if let Some(beat) = self.cue.get_beat(self.state.beat_idx) {
            scheduled_time = self.last_beat_time + beat.length as u64
        } else {
            scheduled_time = u64::MAX
        };
        self.transport.us_to_next_beat =
            if scheduled_time > ctx.jack_time && scheduled_time < u64::MAX / 2 {
                (scheduled_time - ctx.jack_time) as usize
            } else {
                0
            };
        return AudioSourceState::BeatStatus(self.state.clone());
    }

    fn send_buffer(
        &mut self,
        ctx: &audio::source::AudioSourceContext,
    ) -> Result<&[f32], jack::Error> {
        if ctx.transport.running {
            let mut beat = self.cue.get_beat(self.state.beat_idx).unwrap_or_default();
            let next_beat = match self.cue.get_beat(self.state.next_beat_idx) {
                None => {
                    return Ok(self.silence(ctx.frame_size));
                }
                Some(val) => val,
            };
            let scheduled_time: u64 = self.last_beat_time + beat.length as u64;

            if ctx.jack_time > scheduled_time {
                self.state.beat_idx = self.state.next_beat_idx;
                beat = self.cue.get_beat(self.state.beat_idx).unwrap_or_default();
                self.state.next_beat_idx += 1;
                if self.last_beat_time == 0 {
                    self.last_beat_time = ctx.jack_time;
                } else {
                    self.last_beat_time = scheduled_time;
                }
                for event in beat.events {
                    self.handle_event(event, ctx);
                }
                ctx.cbnet.notify(common::status::Notification::BeatChanged(
                    self.state.clone(),
                ));
                return Ok(
                    &self.click_buffers[if beat.count == 1 { 0 } else { 1 }][0..ctx.frame_size]
                );
            } else {
                return Ok(self.silence(ctx.frame_size));
            }
        }
        return Ok(self.silence(ctx.frame_size));
    }

    fn command(
        &mut self,
        ctx: &AudioSourceContext,
        command: ControlCommand,
    ) -> Result<(), CommandError> {
        match command {
            ControlCommand::LoadCue(cue) => {
                self.cue = cue;
            }
            ControlCommand::TransportZero => {
                self.state.beat_idx = 0;
                self.state.next_beat_idx = 0;
                self.last_beat_time = 0;
            }
            ControlCommand::TransportStop => {
                self.last_beat_time = 0;
            }
            ControlCommand::TransportSeekBeat(beat_idx) => {
                self.state.next_beat_idx = beat_idx;
            }
            ControlCommand::TransportJumpBeat(beat_idx) => {
                self.state.next_beat_idx = beat_idx;
                self.last_beat_time = 0;
            }
            _ => {}
        }
        return Ok(());
    }
}
