use crate::audio;
use crate::audio::source::AudioSourceContext;
use common::event::{EventDescription, JumpRequirement};
use common::local::status::{AudioSourceState, BeatState, TransportState};
use common::protocol::message::{Message, SmallMessage};
use common::protocol::request::ControlAction;

struct MetronomeClick {
    frequency: usize,
    length: usize,
}

pub struct Metronome {
    clicks: Vec<MetronomeClick>,
    click_buffers: [[f32; 96000]; 2],
    last_beat_time: u64,
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
        met
    }

    pub fn pregen_click_bufs(&mut self) {
        for i in 0..2 {
            let click = &self.clicks[i];
            let mut buf = [0f32; 96000];
            for (i, sample) in buf.iter_mut().enumerate().take(click.length * 48) {
                *sample =
                    (i as f32 * std::f32::consts::PI * click.frequency as f32 / 24000.0).sin() * 0.1
            }
            self.click_buffers[i] = buf;
        }
    }
}

impl audio::source::AudioSource for Metronome {
    fn get_status(&mut self, ctx: &audio::source::AudioSourceContext) -> AudioSourceState {
        let scheduled_time: u64;
        if let Some(beat) = ctx.cue.get_beat(self.state.beat_idx) {
            scheduled_time = self.last_beat_time
                + (beat.length as u64 * 100 / ctx.transport.playrate_percent as u64)
        } else {
            scheduled_time = u64::MAX
        };
        self.state.us_to_next_beat =
            if scheduled_time > ctx.jack_time && scheduled_time < u64::MAX / 2 {
                (scheduled_time - ctx.jack_time) as u32
            } else {
                0
            };
        AudioSourceState::BeatStatus(self.state)
    }

    fn send_buffer(
        &mut self,
        ctx: &audio::source::AudioSourceContext,
    ) -> Result<&[f32], jack::Error> {
        if ctx.transport.running {
            let mut beat = ctx.cue.get_beat(self.state.beat_idx).unwrap_or_default();
            let next_beat = match ctx.cue.get_beat(self.state.next_beat_idx) {
                None => {
                    return Ok(self.silence(ctx.frame_size));
                }
                Some(val) => val,
            };
            let scheduled_time: u64 = self.last_beat_time
                + beat.length as u64 * 100 / ctx.transport.playrate_percent as u64;

            if ctx.jack_time > scheduled_time {
                self.state.beat_idx = self.state.next_beat_idx;
                beat = ctx.cue.get_beat(self.state.beat_idx).unwrap_or_default();
                self.state.next_beat_idx += 1;
                if self.last_beat_time == 0 {
                    self.last_beat_time = ctx.jack_time;
                } else {
                    self.last_beat_time = scheduled_time;
                }
                //ctx.cbnet
                //    .notify(Message::Small(SmallMessage::BeatData(self.state)));
                return Ok(
                    &self.click_buffers[if beat.count == 1 { 0 } else { 1 }][0..ctx.frame_size]
                );
            } else {
                return Ok(self.silence(ctx.frame_size));
            }
        }
        Ok(self.silence(ctx.frame_size))
    }

    fn command(&mut self, _ctx: &AudioSourceContext, command: ControlAction) {
        match command {
            ControlAction::TransportZero => {
                self.state.beat_idx = 0;
                self.state.next_beat_idx = 0;
                self.last_beat_time = 0;
            }
            ControlAction::TransportStop => {
                self.last_beat_time = 0;
            }
            ControlAction::TransportSeekBeat(beat_idx) => {
                self.state.next_beat_idx = beat_idx;
            }
            ControlAction::TransportJumpBeat(beat_idx) => {
                self.state.next_beat_idx = beat_idx;
                self.last_beat_time = 0;
            }
            _ => {}
        }
    }

    fn event_occured(&mut self, ctx: &AudioSourceContext, event: common::event::Event) {
        if let Some(EventDescription::JumpEvent {
            destination,
            requirement,
            when_jumped,
            when_passed,
        }) = event.event
        {
            let requirement_fullfilled = match requirement {
                JumpRequirement::JumpModeOn => ctx.transport.vlt,
                JumpRequirement::JumpModeOff => !ctx.transport.vlt,
                JumpRequirement::None => true,
            };

            if requirement_fullfilled {
                self.state.next_beat_idx = destination;
                self.state.requested_vlt_action = when_jumped;
            } else {
                self.state.requested_vlt_action = when_passed;
            }
        }
    }

    fn event_will_occur(&mut self, _ctx: &AudioSourceContext, _event: common::event::Event) {}
}
