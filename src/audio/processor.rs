use crossbeam_channel::{Receiver, Sender};
use jack::{
    AudioIn, AudioOut, Client, Control, NotificationHandler, Port, PortFlags, ProcessHandler,
    ProcessScope, Unowned,
};

use crate::{audio::source::SourceConfig, logger, CrossbeamNetwork};

use common::{
    command::ControlCommand,
    network::{JACKStatus, StatusMessageKind},
    status::{AudioSourceStatus, CombinedStatus, ProcessStatus},
};

pub struct AudioProcessor {
    sources: Vec<SourceConfig>,
    cbnet: CrossbeamNetwork,
    status: CombinedStatus,
    ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
}

impl AudioProcessor {
    pub fn new(
        sources: Vec<SourceConfig>,
        ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
        cbnet: CrossbeamNetwork,
    ) -> AudioProcessor {
        AudioProcessor {
            ports,
            sources,
            cbnet,
            status: CombinedStatus {
                process_status: ProcessStatus {
                    gains: vec![0.0f32; 64],
                    ..Default::default()
                },
                ..Default::default()
            },
        }
    }
}

impl ProcessHandler for AudioProcessor {
    fn process(&mut self, c: &Client, ps: &ProcessScope) -> Control {
        // Handle channel commands
        loop {
            match self.cbnet.cmd_rx.try_recv() {
                Ok(cmd) => {
                    logger::log(
                        format!("ControlCommand: {cmd}"),
                        logger::LogContext::AudioProcessor,
                        logger::LogKind::Command,
                    );
                    match cmd.clone() {
                        ControlCommand::TransportStart => {
                            self.status.process_status.running = true;
                        }
                        ControlCommand::TransportStop => {
                            self.status.process_status.running = false;
                        }

                        ControlCommand::LoadShow(show) => {
                            self.status.show = show;
                            let _ = self
                                .cbnet
                                .cmd_tx
                                .try_send(ControlCommand::LoadCueByIndex(0));
                        }

                        ControlCommand::LoadCue(cue) => {
                            self.status.process_status.running = false;
                            self.status.cue = cue.clone();

                            let _ = self.cbnet.cmd_tx.try_send(ControlCommand::TransportStop);
                            let _ = self.cbnet.cmd_tx.try_send(ControlCommand::TransportZero);
                            let _ =
                                self.cbnet
                                    .status_tx
                                    .try_send(StatusMessageKind::CueStatus(Some(
                                        self.status.cue.clone(),
                                    )));
                        }
                        ControlCommand::LoadCueFromSelfIndex => {
                            let _ = self.cbnet.cmd_tx.try_send(ControlCommand::LoadCue(
                                self.status.show.cues[self.status.process_status.cue_idx].clone(),
                            ));
                        }
                        ControlCommand::LoadCueByIndex(idx) => {
                            if idx < self.status.show.cues.len() {
                                self.status.process_status.cue_idx = idx;
                                let _ = self
                                    .cbnet
                                    .cmd_tx
                                    .try_send(ControlCommand::LoadCueFromSelfIndex);
                            }
                        }
                        ControlCommand::LoadPreviousCue => {
                            if self.status.process_status.cue_idx > 0 {
                                self.status.process_status.cue_idx += 1;
                                let _ = self
                                    .cbnet
                                    .cmd_tx
                                    .try_send(ControlCommand::LoadCueFromSelfIndex);
                            }
                        }
                        ControlCommand::LoadNextCue => {
                            if self.status.process_status.cue_idx + 1 < self.status.show.cues.len()
                            {
                                self.status.process_status.cue_idx += 1;
                                let _ = self
                                    .cbnet
                                    .cmd_tx
                                    .try_send(ControlCommand::LoadCueFromSelfIndex);
                            }
                        }
                        ControlCommand::DumpStatus => {
                            let _ =
                                self.cbnet
                                    .status_tx
                                    .try_send(StatusMessageKind::CueStatus(Some(
                                        self.status.cue.clone(),
                                    )));
                            let _ =
                                self.cbnet
                                    .status_tx
                                    .try_send(StatusMessageKind::ShowStatus(Some(
                                        self.status.show.lightweight(),
                                    )));
                        }

                        ControlCommand::SetChannelGain(channel_idx, gain) => {
                            self.sources[channel_idx].set_gain(gain);
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
                Err(err) => logger::log(
                    format!("Error reading command: {}", err),
                    logger::LogContext::AudioProcessor,
                    logger::LogKind::Error,
                ),
            }
        }

        // Get status from all sources and compile onto self.status
        let mut source_statuses: Vec<AudioSourceStatus> = vec![]; // stati??
        for (i, source) in &mut self.sources.iter_mut().enumerate() {
            self.status.process_status.gains[i] = source.get_gain();
            let status = source.source_device.get_status(c, ps);
            match status {
                AudioSourceStatus::BeatStatus(ref status) => {
                    self.status.process_status.beat_idx = status.beat_idx;
                    self.status.process_status.us_to_next_beat = status.us_to_next;
                    self.status.process_status.next_beat_idx = status.next_beat_idx;
                }
                AudioSourceStatus::TimeStatus(ref status) => {
                    self.status.process_status.time = status.clone();
                }
                _ => {}
            }
            source_statuses.push(status);
        }

        if self.status.process_status.next_beat_idx >= self.status.cue.get_beats().len()
            && self.status.process_status.running
        {
            self.status.process_status.running = false;
            self.cbnet.cmd_tx.try_send(ControlCommand::TransportStop);
            self.cbnet.cmd_tx.try_send(ControlCommand::LoadNextCue);
            self.cbnet.cmd_tx.try_send(ControlCommand::TransportZero);
        }

        self.status.process_status.sources = source_statuses;
        // Get audio frame buffers from all children and play in correct port
        for i in 0..self.sources.len() {
            let source = &mut self.sources[i];
            let res = source
                .source_device
                .send_buffer(c, ps, self.status.process_status.clone());
            if let Ok(buf) = res {
                let mut out_buf = self.ports.0[i].as_mut_slice(ps);
                out_buf.clone_from_slice(buf);
                for i in 0..out_buf.len() {
                    out_buf[i] *= source.get_gain_mult().clone();
                }
            } else {
                logger::log(
                    format!("Audio error occured in source {}.", i),
                    logger::LogContext::AudioProcessor,
                    logger::LogKind::Error,
                );
                return Control::Quit;
            }
        }

        self.status.process_status.system_time_us =
            chrono::prelude::Utc::now().timestamp_micros() as u64;
        self.status.process_status.cpu_use = c.cpu_load();

        let _ = self
            .cbnet
            .status_tx
            .try_send(StatusMessageKind::ProcessStatus(Some(
                self.status.process_status.clone(),
            )));

        return Control::Continue;
    }
}
