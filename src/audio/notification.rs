use jack::NotificationHandler;

pub struct JACKNotificationHandler;
impl NotificationHandler for JACKNotificationHandler {
    //fn thread_init(&self, _: &Client) {}
    //unsafe fn shutdown(&mut self, _status: ClientStatus, _reason: &str) {}
    //fn freewheel(&mut self, _: &Client, _is_freewheel_enabled: bool) {}
    //fn sample_rate(&mut self, _: &Client, _srate: Frames) -> Control {}
    //fn client_registration(&mut self, _: &Client, _name: &str, _is_registered: bool) {}
    //fn port_registration(&mut self, _: &Client, _port_id: PortId, _is_registered: bool) {}
    //fn port_rename(
    //    &mut self,
    //    _: &Client,
    //    _port_id: PortId,
    //    _old_name: &str,
    //    _new_name: &str,
    //) -> Control {
    //}
    //fn ports_connected(
    //    &mut self,
    //    _: &Client,
    //    _port_id_a: PortId,
    //    _port_id_b: PortId,
    //    _are_connected: bool,
    //) {
    //}
    //fn graph_reorder(&mut self, _: &Client) -> Control {}
    //fn xrun(&mut self, _: &Client) -> Control {}
}
