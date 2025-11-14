use crate::{
    audio::{
        notification::JACKNotificationHandler, processor::AudioProcessor, source::SourceConfig,
    },
    logger, CrossbeamNetwork,
};
use common::{
    config::AudioConfiguration,
    network::{AudioDevice, JACKStatus},
    status::Notification,
};
use jack::{AsyncClient, AudioOut, Client, ClientOptions, Port, PortFlags, Unowned};

pub struct AudioHandler {
    pub client: Option<AsyncClient<JACKNotificationHandler, AudioProcessor>>,
    pub num_sources: usize,
    config: AudioConfiguration,
    jack_server_process: Option<std::process::Child>,
    cbnet: CrossbeamNetwork,
    pub jack_status: JACKStatus,
}

impl AudioHandler {
    pub fn new(num_sources: usize, cbnet: CrossbeamNetwork) -> AudioHandler {
        AudioHandler {
            jack_status: JACKStatus::default(),
            cbnet,
            client: None,
            num_sources,
            config: AudioConfiguration::default(),
            jack_server_process: None,
        }
    }

    pub fn configure(&mut self, config: AudioConfiguration) {
        self.config = config
    }

    pub fn start(&mut self, sources: Vec<SourceConfig>) {
        self.start_server();
        std::thread::sleep(std::time::Duration::from_secs(5));
        let client_res = self.start_client();
        let client = match client_res {
            Err(err) => {
                logger::log(
                    format!("Could not open JACK client: {:#?}", err),
                    common::config::LogContext::AudioHandler,
                    common::config::LogKind::Error,
                );
                self.shutdown();
                return;
            }
            Ok(client) => client,
        };
        let mut ports: (Vec<Port<AudioOut>>, Vec<Port<Unowned>>) = (vec![], vec![]);
        ports.0 = self.init_client_ports(&client);
        ports.1 = self.collect_system_ports(&client);

        let processor = AudioProcessor::new(sources, ports, self.cbnet.clone());
        let ac = match client.activate_async(JACKNotificationHandler, processor) {
            Ok(val) => val,
            Err(err) => {
                logger::log(
                    format!("Error starting audio client: {err}"),
                    common::config::LogContext::AudioHandler,
                    common::config::LogKind::Error,
                );
                return;
            }
        };
        self.client = Some(ac);
    }

    fn get_ports(&self) -> (Vec<Port<Unowned>>, Vec<Port<Unowned>>) {
        if self.client.is_none() {
            return (vec![], vec![]);
        }
        let mut ports: (Vec<Port<Unowned>>, Vec<Port<Unowned>>) = (vec![], vec![]);
        let client = self
            .client
            .as_ref()
            .expect("Client is none is handled.")
            .as_client();
        let mut i_ports = client.ports(
            Some(&self.config.client.name),
            Some("32 bit float mono audio"),
            PortFlags::IS_OUTPUT,
        );
        i_ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        ports.0 = i_ports
            .iter()
            .map(|name| {
                client
                    .port_by_name(name)
                    .expect("Port names are unexposed and should never be incorrect.")
            })
            .collect();

        let mut o_ports = client.ports(
            Some(&self.config.server.system_name),
            Some("32 bit float mono audio"),
            PortFlags::IS_INPUT,
        );
        o_ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        ports.1 = o_ports
            .iter()
            .map(|name| {
                client
                    .port_by_name(name)
                    .expect("Port names are unexposed and should never be incorrect.")
            })
            .collect();

        return ports;
    }

    pub fn get_connections(&self) -> Vec<(usize, usize)> {
        let ports = self.get_ports();
        return ports
            .0
            .iter()
            .enumerate()
            .map(|(a_idx, a_port)| {
                ports
                    .1
                    .iter()
                    .enumerate()
                    .map(|(b_idx, b_port)| {
                        if a_port
                            .is_connected_to(
                                &b_port.name().expect(
                                    "Port names are unexposed and should never be incorrect.",
                                ),
                            )
                            .unwrap_or_default()
                        {
                            (a_idx, b_idx)
                        } else {
                            (usize::MAX, usize::MAX)
                        }
                    })
                    .collect::<Vec<(usize, usize)>>()
            })
            .flatten()
            .filter(|(a, b)| *a < usize::MAX && *b < usize::MAX)
            .collect();
    }

    pub fn try_route_ports(&mut self, from: usize, to: usize, connect: bool) -> bool {
        let ports = self.get_ports();
        let p_from = ports.0[from].clone();
        let p_to = ports.1[to].clone();
        let client = match &self.client {
            Some(val) => val.as_client(),
            None => return false,
        };
        let res = if connect {
            client.connect_ports(&p_from, &p_to)
        } else {
            client.disconnect_ports(&p_from, &p_to)
        };

        match res {
            Ok(val) => {
                logger::log(
                    format!("Set port [{from}] -> [{to}] to {connect}"),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Note,
                );

                return true;
            }
            Err(err) => {
                logger::log(
                    format!("JACK Connection Error: {err}"),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Error,
                );
                return false;
            }
        }
    }

    pub fn get_jack_status(&mut self) -> JACKStatus {
        self.jack_status.running = !self.client.is_none();
        self.jack_status.available_devices = self.get_hw_devices();

        if self.jack_status.running {
            let client = match &self.client {
                Some(val) => val.as_client(),
                None => return self.jack_status.clone(),
            };
            self.jack_status.io_size = (self.get_ports().0.len(), self.get_ports().1.len());
            self.jack_status.buffer_size = client.buffer_size() as usize;
            self.jack_status.sample_rate = client.sample_rate();
            self.jack_status.frame_size = 0;
            self.jack_status.client_name = client.name().to_string();
            self.jack_status.output_name = self.config.server.system_name.clone();
            self.jack_status.connections = self.get_connections();
        }
        return self.jack_status.clone();
    }

    pub fn shutdown(&mut self) {
        match self.jack_server_process.as_mut() {
            None => return,
            Some(val) => val.kill(),
        };
    }

    pub fn start_server(&mut self) {
        self.jack_server_process = match std::process::Command::new("jackd")
            .arg("-R")
            .args(["-d", "alsa"])
            .args(["-d", &self.config.server.device_id])
            .args(["-r", &self.config.server.sample_rate.to_string()])
            .spawn()
        {
            Ok(val) => Some(val),
            Err(err) => None,
        };
    }

    pub fn start_client(&mut self) -> Result<Client, jack::Error> {
        let client_res = Client::new(
            &self.config.client.name.to_string(),
            ClientOptions::NO_START_SERVER,
        );
        match client_res {
            Err(err) => Err(err),
            Ok((client, status)) => {
                logger::log(
                    format!("Opened JACK client ({status:?})"),
                    logger::LogContext::AudioHandler,
                    logger::LogKind::Note,
                );
                Ok(client)
            }
        }
    }

    pub fn init_client_ports(&self, client: &Client) -> Vec<Port<AudioOut>> {
        let mut ports = vec![];
        // Register io_matrix.0 amount of ports on the client and save for processor
        // reference
        for c_out_idx in 0..self.num_sources {
            ports.push(
                client
                    .register_port(&c_out_idx.to_string(), AudioOut::default())
                    .expect("Port register failed"),
            );
        }
        return ports;
    }

    pub fn collect_system_ports(&self, client: &Client) -> Vec<Port<Unowned>> {
        let mut ports = client.ports(
            Some(&self.config.server.system_name),
            Some("32 bit float mono audio"),
            PortFlags::IS_INPUT,
        );
        ports.sort_by_key(|name| {
            let mut new_name = name.clone();
            new_name.retain(|c| c.is_numeric());
            return new_name.parse::<usize>().unwrap_or_default();
        });
        logger::log(
            format!("Found {} system ports.", ports.len()),
            logger::LogContext::AudioHandler,
            logger::LogKind::Note,
        );
        return ports
            .iter()
            .map(|name| {
                client
                    .port_by_name(name)
                    .expect("Port names are unexposed and should always be right")
            })
            .collect();
    }

    pub fn send_status(&mut self) {
        let status = self.get_jack_status();
        let _ = self.cbnet.notify(Notification::JACKStateChanged(status));
    }

    pub fn get_hw_devices(&self) -> Vec<AudioDevice> {
        let output = std::process::Command::new("aplay")
            .arg("--list-devices")
            .output()
            .expect("Failed to execute `aplay`");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut devices = Vec::new();

        for line in stdout.lines() {
            if line.trim_start().starts_with("card") {
                if let Some(device) = AudioDevice::from_aplay_str(line.to_string()) {
                    devices.push(device);
                }
            }
        }

        devices
    }

    pub fn get_cpu_use(&self) -> f32 {
        match &self.client {
            Some(val) => val.as_client().cpu_load(),
            None => 0.0,
        }
    }
}
