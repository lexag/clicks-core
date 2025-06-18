use crossbeam_channel::{Receiver, Sender};
use jack::{AudioOut, Client, Control, Port, ProcessHandler, ProcessScope};
use serde::Serialize;

use crate::audio::source::SourceConfig;

use common::{
    command::ControlCommand,
    cue::Cue,
    status::{AudioSourceStatus, ProcessStatus},
};

pub struct AudioProcessor {
    sources: Vec<SourceConfig>,
    ports: Vec<Port<AudioOut>>,
    tx: Sender<ProcessStatus>,
    rx: Receiver<ControlCommand>,
    status: ProcessStatus,
}

impl AudioProcessor {
    pub fn new(
        sources: Vec<SourceConfig>,
        ports: Vec<Port<AudioOut>>,
        rx: Receiver<ControlCommand>,
        tx: Sender<ProcessStatus>,
    ) -> AudioProcessor {
        AudioProcessor {
            sources,
            ports,
            tx,
            rx,
            status: ProcessStatus {
                ..Default::default()
            },
        }
    }
}

impl ProcessHandler for AudioProcessor {
    fn process(&mut self, c: &Client, ps: &ProcessScope) -> Control {
        // Handle channel commands
        loop {
            match self.rx.try_recv() {
                Ok(cmd) => {
                    println!("RCVCMD: {:?}", cmd);
                    match &cmd {
                        ControlCommand::TransportStart => {
                            self.status.running = true;
                        }
                        ControlCommand::TransportStop => {
                            self.status.running = false;
                        }
                        ControlCommand::LoadCue(cue) => self.status.cue = cue.clone(),
                        _ => {}
                    }

                    // Pass on commands to all children
                    // to do source specific implementations
                    for source in &mut self.sources {
                        let _ = source.source_device.command(cmd.clone());
                    }
                }

                // If channel is empty, continue with process
                Err(crossbeam_channel::TryRecvError::Empty) => {
                    break;
                }
                Err(err) => println!("CMDERR: {}", err),
            }
        }

        // Get JACK Transport time ( baseline time for all time-timebase sources
        let state = c.transport().query().unwrap();
        let sample_time = state.pos.frame() as u64;
        let sample_rate = state.pos.frame_rate().unwrap() as u64;
        let t_us: u128 = ((sample_time << 16) / sample_rate) as u128;
        self.status.h = ((t_us >> 16) / 3600).try_into().unwrap();
        self.status.m = ((t_us >> 16) / 60 % 60).try_into().unwrap();
        self.status.s = ((t_us >> 16) % 60).try_into().unwrap();

        // Get status from all sources and compile onto self.status
        let mut source_statuses: Vec<AudioSourceStatus> = vec![]; // stati??
        for source in &mut self.sources {
            let status = source.source_device.get_status(c, ps);
            match status {
                AudioSourceStatus::BeatStatus(ref status) => {
                    self.status.beat_idx = status.beat_idx;
                    self.status.us_to_next_beat = status.us_to_next;
                    self.status.next_beat_idx = status.next_beat_idx;
                }
                _ => {}
            }
            source_statuses.push(status);
        }

        self.status.sources = source_statuses;
        // Get audio frame buffers from all children and play in correct port
        for i in 0..self.sources.len() {
            let source = &mut self.sources[i];
            let res = source.source_device.send_buffer(c, ps, self.status.clone());
            if let Ok(buf) = res {
                self.ports[i].as_mut_slice(ps).clone_from_slice(buf);
            } else {
                println!("Audio error occured in source {}", i);
                return Control::Quit;
            }
        }

        let _ = self.tx.try_send(self.status.clone());

        //println!("{:?}", process_status);
        return Control::Continue;
    }
}
