use crossbeam_channel::{Receiver, Sender};
use jack::{
    AudioIn, AudioOut, Client, Control, NotificationHandler, Port, PortFlags, ProcessHandler,
    ProcessScope, Unowned,
};

use crate::{
    audio::source::{AudioSource, SourceConfig},
    logger, CrossbeamNetwork,
};

use common::{
    command::ControlCommand,
    network::{Heartbeat, JACKStatus},
    status::{AudioSourceState, CombinedStatus, Notification, NotificationKind},
};

pub struct AudioProcessor {
    sources: Vec<SourceConfig>,
    cbnet: CrossbeamNetwork,
    status: CombinedStatus,
    ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
    status_changed_flag: bool,
}

impl AudioProcessor {
    pub fn new(
        sources: Vec<SourceConfig>,
        ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>),
        cbnet: CrossbeamNetwork,
    ) -> AudioProcessor {
        let mut ap = AudioProcessor {
            ports,
            sources,
            cbnet,
            status: CombinedStatus::default(),
            status_changed_flag: false,
        };
        ap.status.sources = vec![AudioSourceState::None; 32];
        ap
    }

    fn send_all_status(&self) {
        let _ = self
            .cbnet
            .notify(Notification::CueChanged(self.status.cue.clone()));
        let _ = self
            .cbnet
            .notify(Notification::ShowChanged(self.status.show.lightweight()));
        let _ = self
            .cbnet
            .notify(Notification::BeatChanged(self.status.beat_state().clone()));
        let _ = self.cbnet.notify(Notification::TransportChanged(
            self.status.transport.clone(),
        ));
    }

    fn notify_push(&mut self, notification_kind: NotificationKind) {
        self.cbnet.notify(match notification_kind {
            NotificationKind::TransportChanged => {
                Notification::TransportChanged(self.status.transport.clone())
            }
            NotificationKind::BeatChanged => Notification::BeatChanged(self.status.beat_state()),
            NotificationKind::CueChanged => Notification::CueChanged(self.status.cue.clone()),
            NotificationKind::ShowChanged => Notification::ShowChanged(self.status.show.clone()),
            _ => {
                return;
            }
        });
    }

    fn handle_command(&mut self, command: ControlCommand) {
        logger::log(
            format!("ControlCommand: {command}"),
            logger::LogContext::AudioProcessor,
            logger::LogKind::Command,
        );
        match command.clone() {
            ControlCommand::DumpStatus => self.send_all_status(),
            ControlCommand::TransportStart => {
                self.status.transport.running = true;
                self.notify_push(NotificationKind::TransportChanged);
            }
            ControlCommand::TransportStop => {
                self.status.transport.running = false;
                self.notify_push(NotificationKind::TransportChanged);
            }

            ControlCommand::TransportSeekBeat(..) | ControlCommand::TransportJumpBeat(..) => {
                self.status_changed_flag = true;
            }

            ControlCommand::LoadShow(show) => {
                self.status.show = show;
                self.cbnet.command(ControlCommand::LoadCueByIndex(0));
                self.notify_push(NotificationKind::ShowChanged);
            }

            ControlCommand::LoadCue(cue) => {
                self.status.transport.running = false;
                self.status.cue.cue = cue.clone();

                self.cbnet.command(ControlCommand::TransportStop);
                self.cbnet.command(ControlCommand::TransportZero);
                self.notify_push(NotificationKind::CueChanged);
            }
            ControlCommand::LoadCueFromSelfIndex => {
                let _ = self.cbnet.command(ControlCommand::LoadCue(
                    self.status.show.cues[self.status.cue.cue_idx].clone(),
                ));
            }
            ControlCommand::LoadCueByIndex(idx) => {
                if idx < self.status.show.cues.len() {
                    self.status.cue.cue_idx = idx;
                    let _ = self.cbnet.command(ControlCommand::LoadCueFromSelfIndex);
                }
            }
            ControlCommand::LoadPreviousCue => {
                if self.status.cue.cue_idx > 0 {
                    self.status.cue.cue_idx -= 1;
                    let _ = self.cbnet.command(ControlCommand::LoadCueFromSelfIndex);
                }
            }
            ControlCommand::LoadNextCue => {
                if self.status.cue.cue_idx + 1 < self.status.show.cues.len() {
                    self.status.cue.cue_idx += 1;
                    let _ = self.cbnet.command(ControlCommand::LoadCueFromSelfIndex);
                }
            }

            ControlCommand::SetChannelGain(channel_idx, gain) => {
                self.sources[channel_idx].set_gain(gain);
            }

            _ => {}
        }

        // Pass on commands to all children
        // to do source specific implementations
        for source in &mut self.sources {
            let _ = source.source_device.command(command.clone());
        }
    }

    fn compile_child_statuses(&mut self, c: &Client, ps: &ProcessScope) {
        for (i, source) in self.sources.iter_mut().enumerate() {
            let status = source.source_device.get_status(c, ps);
            self.status.sources[i] = status;
        }
    }

    // Get audio buffer from source[idx] and copy it to the JACK client output buffer.
    fn process_child(&mut self, idx: usize, c: &Client, ps: &ProcessScope) -> Control {
        let source = &mut self.sources[idx];
        let res = source.source_device.send_buffer(c, ps, self.status.clone());
        if let Ok(buf) = res {
            let mut out_buf = self.ports.0[idx].as_mut_slice(ps);
            out_buf.clone_from_slice(buf);
            for i in 0..out_buf.len() {
                out_buf[i] *= source.get_gain_mult().clone();
            }
            return Control::Continue;
        } else {
            logger::log(
                format!("Audio error occured in source {}.", idx),
                logger::LogContext::AudioProcessor,
                logger::LogKind::Error,
            );
            return Control::Quit;
        }
    }
}

impl ProcessHandler for AudioProcessor {
    fn process(&mut self, c: &Client, ps: &ProcessScope) -> Control {
        // Handle channel commands
        loop {
            match self.cbnet.cmd_rx.try_recv() {
                Ok(cmd) => self.handle_command(cmd),

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
        self.compile_child_statuses(c, ps);

        // If cue runs out: stop and go to next
        if self.status.beat_state().beat_idx >= self.status.cue.cue.get_beats().len()
            && self.status.transport.running
        {
            self.status.transport.running = false;
            self.cbnet.command(ControlCommand::TransportStop);
            self.cbnet.command(ControlCommand::LoadNextCue);
            self.cbnet.command(ControlCommand::TransportZero);
        }

        // Get audio frame buffers from all children and play in correct port
        for i in 0..self.sources.len() {
            if self.process_child(i, c, ps) == Control::Quit {
                return Control::Quit;
            };
        }

        if self.status.transport.running || self.status_changed_flag {
            self.notify_push(NotificationKind::TransportChanged);
            self.status_changed_flag = false;
        }

        return Control::Continue;
    }
}
