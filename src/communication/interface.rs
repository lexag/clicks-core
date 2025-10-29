use common::protocol::{message::Message, request::Request};

pub trait CommunicationInterface: Send {
    fn get_inputs(&mut self, limit: usize) -> Vec<Request>;

    fn get_all_inputs(&mut self) -> Vec<Request> {
        return self.get_inputs(usize::MAX);
    }

    fn get_single_input(&mut self) -> Option<Request> {
        return self.get_inputs(1).get(0).cloned();
    }

    fn notify(&mut self, message: Message);

    fn notify_multiple(&mut self, messages: Vec<Message>);
}
