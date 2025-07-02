use common::status::BeatStatus;
use jack::{Client, ProcessScope};

use crate::audio;
use common::command::{CommandError, ControlCommand};
use common::{
    cue::{BeatEvent, Cue},
    status::{AudioSourceStatus, ProcessStatus},
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
    beat_idx: usize,
    next_beat_idx: usize,
    status: ProcessStatus,
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
            beat_idx: 0,
            next_beat_idx: 1,
            status: ProcessStatus::default(),
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
    fn handle_event(&mut self, event: BeatEvent) {
        match event {
            BeatEvent::JumpEvent { destination } => self.next_beat_idx = destination,
            BeatEvent::VampEvent { length } => self.next_beat_idx -= length,
            _ => {}
        }
    }
}

impl audio::source::AudioSource for Metronome {
    fn get_status(&mut self, c: &Client, _ps: &ProcessScope) -> AudioSourceStatus {
        let t_us = c.frames_to_time(c.frame_time());
        let next_schd_t_us: u64;
        if let Ok(beat) = self.cue.get_beat(self.beat_idx) {
            next_schd_t_us = self.last_beat_time + (beat.length * 1000) as u64
        } else {
            next_schd_t_us = u64::MAX
        };
        return AudioSourceStatus::BeatStatus(BeatStatus {
            beat_idx: self.beat_idx,
            next_beat_idx: self.next_beat_idx,
            us_to_next: if next_schd_t_us > t_us {
                (next_schd_t_us - t_us) as usize
            } else {
                0
            },
        });
    }
    fn send_buffer(
        &mut self,
        c: &Client,
        ps: &ProcessScope,
        status: ProcessStatus,
    ) -> Result<&[f32], jack::Error> {
        self.status = status.clone();
        if status.running {
            let res = self.cue.get_beat(self.next_beat_idx);
            if let Err(err) = res {
                return Ok(&[0f32; 2048][0..ps.n_frames() as usize]);
            }
            let next_beat = res.unwrap();
            let t_us = c.frames_to_time(c.frame_time());
            let next_schd_t_us: u64 = self.last_beat_time + (next_beat.length * 1000) as u64;

            //println!(
            //    "t: {}, sch: {}, lb: {}",
            //    t_us / 1000,
            //    next_schd_t_us / 1000,
            //    self.last_beat_time / 1000
            //);
            if t_us > next_schd_t_us {
                self.beat_idx = self.next_beat_idx;
                self.next_beat_idx += 1;
                if self.last_beat_time == 0 {
                    self.last_beat_time = t_us;
                } else {
                    self.last_beat_time = next_schd_t_us;
                }
                let beat = self.cue.get_beat(self.beat_idx).unwrap();
                for event in beat.events {
                    self.handle_event(event);
                }
                return Ok(&self.click_buffers[if beat.count == 1 { 0 } else { 1 }]
                    [0..ps.n_frames() as usize]);
            } else {
                return Ok(&[0f32; 2048][0..ps.n_frames() as usize]);
            }
        }
        return Ok(&[0f32; 2048][0..ps.n_frames() as usize]);
    }

    fn command(&mut self, command: ControlCommand) -> Result<(), CommandError> {
        match command {
            ControlCommand::LoadCue(cue) => {
                self.cue = cue;
            }
            ControlCommand::TransportZero => {
                self.beat_idx = usize::MAX;
                self.next_beat_idx = 0;
                self.last_beat_time = 0;
            }
            ControlCommand::TransportStop => {
                self.last_beat_time = 0;
            }
            ControlCommand::TransportSeekBeat(beat_idx) => {
                self.next_beat_idx = beat_idx;
            }
            ControlCommand::TransportJumpBeat(beat_idx) => {
                self.next_beat_idx = beat_idx;
                self.last_beat_time = 0;
            }
            _ => {}
        }
        return Ok(());
    }
}
