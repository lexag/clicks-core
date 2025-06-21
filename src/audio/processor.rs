use crossbeam_channel::{Receiver, Sender};
use jack::{AudioOut, Client, Control, Port, ProcessHandler, ProcessScope};

use crate::audio::source::SourceConfig;

use common::{
    command::ControlCommand,
    network::{JACKStatus, StatusMessageKind},
    status::{AudioSourceStatus, CombinedStatus},
};

pub struct AudioProcessor {
    sources: Vec<SourceConfig>,
    ports: Vec<Port<AudioOut>>,
    tx: Sender<StatusMessageKind>,
    rx: Receiver<ControlCommand>,
    status: CombinedStatus,
}

impl AudioProcessor {
    pub fn new(
        sources: Vec<SourceConfig>,
        ports: Vec<Port<AudioOut>>,
        rx: Receiver<ControlCommand>,
        tx: Sender<StatusMessageKind>,
        jack_status: JACKStatus,
    ) -> AudioProcessor {
        AudioProcessor {
            sources,
            ports,
            tx,
            rx,
            status: CombinedStatus {
                jack_status,
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
                            self.status.process_status.running = true;
                        }
                        ControlCommand::TransportStop => {
                            self.status.process_status.running = false;
                        }
                        ControlCommand::LoadCue(cue) => {
                            self.status.cue = cue.clone();
                            let _ = self.tx.try_send(StatusMessageKind::CueStatus(Some(
                                self.status.cue.clone(),
                            )));
                        }
                        ControlCommand::NotifySubscribers => {
                            let _ = self.tx.try_send(StatusMessageKind::CueStatus(Some(
                                self.status.cue.clone(),
                            )));
                            let _ = self.tx.try_send(StatusMessageKind::ShowStatus(Some(
                                self.status.show.clone(),
                            )));
                            let _ = self.tx.try_send(StatusMessageKind::JACKStatus(Some(
                                self.status.jack_status.clone(),
                            )));
                        }
                        ControlCommand::Shutdown => {
                            let _ = self.tx.try_send(StatusMessageKind::Shutdown);
                        }
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
        self.status.process_status.h = ((t_us >> 16) / 3600).try_into().unwrap();
        self.status.process_status.m = ((t_us >> 16) / 60 % 60).try_into().unwrap();
        self.status.process_status.s = ((t_us >> 16) % 60).try_into().unwrap();

        // Get status from all sources and compile onto self.status
        let mut source_statuses: Vec<AudioSourceStatus> = vec![]; // stati??
        for source in &mut self.sources {
            let status = source.source_device.get_status(c, ps);
            match status {
                AudioSourceStatus::BeatStatus(ref status) => {
                    self.status.process_status.beat_idx = status.beat_idx;
                    self.status.process_status.us_to_next_beat = status.us_to_next;
                    self.status.process_status.next_beat_idx = status.next_beat_idx;
                }
                _ => {}
            }
            source_statuses.push(status);
        }

        self.status.process_status.sources = source_statuses;
        // Get audio frame buffers from all children and play in correct port
        for i in 0..self.sources.len() {
            let source = &mut self.sources[i];
            let res = source
                .source_device
                .send_buffer(c, ps, self.status.process_status.clone());
            if let Ok(buf) = res {
                self.ports[i].as_mut_slice(ps).clone_from_slice(buf);
            } else {
                println!("Audio error occured in source {}", i);
                return Control::Quit;
            }
        }

        let _ = self.tx.try_send(StatusMessageKind::ProcessStatus(Some(
            self.status.process_status.clone(),
        )));

        //println!("{:?}", process_status);
        return Control::Continue;
    }
}
