use common::{control::ControlMessage, status::Notification};

pub trait CommunicationInterface: Send {
    fn get_inputs(&mut self, limit: usize) -> Vec<ControlMessage>;

    fn get_all_inputs(&mut self) -> Vec<ControlMessage> {
        return self.get_inputs(usize::MAX);
    }

    fn get_single_input(&mut self) -> Option<ControlMessage> {
        return self.get_inputs(1).get(0).cloned();
    }

    fn notify(&mut self, notification: Notification);

    fn notify_multiple(&mut self, notifications: Vec<Notification>);
}
