use crate::audio::{
    config::AudioConfig, notification::JACKNotificationHandler, processor::AudioProcessor,
    source::SourceConfig,
};
use common::network::StatusMessageKind;
use crossbeam_channel::{Receiver, Sender};
use jack::{AsyncClient, AudioOut, Client, ClientOptions, Error, PortFlags};

use common::command::ControlCommand;
use common::status::ProcessStatus;

pub struct AudioHandler {
    pub client: AsyncClient<JACKNotificationHandler, AudioProcessor>,
    pub config: AudioConfig,
}

impl AudioHandler {
    pub fn new(
        config: AudioConfig,
        sources: Vec<SourceConfig>,
        rx: Receiver<ControlCommand>,
        tx: Sender<StatusMessageKind>,
    ) -> AudioHandler {
        let client_res = Client::new(
            &config.client_name.to_string(),
            ClientOptions::NO_START_SERVER,
        );
        match client_res {
            Ok((client, status)) => {
                println!("Opened JACK client ({status:?})",);
                let mut ports = vec![];
                let mut connections_to_make = vec![];
                for source in &sources {
                    ports.push(
                        client
                            .register_port(&source.name, AudioOut::default())
                            .unwrap(),
                    );
                    for dest in &source.connections {
                        connections_to_make.push((source.name.clone(), dest.clone()));
                    }
                }

                let processor = AudioProcessor::new(sources, ports, rx, tx);
                let ac = client
                    .activate_async(JACKNotificationHandler, processor)
                    .unwrap();
                let ah = AudioHandler { client: ac, config };

                for (from, to) in connections_to_make {
                    ah.try_connect_ports(from, to);
                }
                return ah;
            }
            Err(err) => {
                panic!("Couldn't start JACK client: {err:?}")
            }
        }
    }

    fn try_connect_ports(&self, from: String, to: String) -> bool {
        if let Err(err) = self.client.as_client().connect_ports_by_name(
            &format!("{}:{from}", self.config.client_name),
            &format!("{}:{to}", self.config.system_name),
        ) {
            match err {
                Error::PortConnectionError {
                    source,
                    destination,
                    code_or_message,
                } => {
                    println!(
                        "JACK Connection Error occured attempting to connect [{source}] to [{destination}]. {code_or_message}"
                    );
                    println!("Available ports are:");
                    self.print_ports();
                    return true;
                }
                _ => {
                    println!("Unhandled JACK error connecting [{from}] to [{to}]");
                    return false;
                }
            }
        } else {
            println!("Connected [{from}] to [{to}]");
            return true;
        }
    }

    fn print_ports(&self) {
        let mut port_names = self.client.as_client().ports(
            Some(self.config.client_name.as_str()),
            Some("audio"),
            PortFlags::IS_OUTPUT,
        );
        port_names.extend_from_slice(&self.client.as_client().ports(
            Some("system"),
            Some("audio"),
            PortFlags::IS_INPUT,
        ));
        for port in port_names {
            println!("{}", port);
        }
    }
}
