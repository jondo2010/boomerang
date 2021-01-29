use super::scheduler::Scheduler;

slotmap::new_key_type!{
    pub struct ReactorKey;
}

pub trait ReactorElement {
    fn startup(&self, _scheduler: &mut Scheduler) {}
    fn shutdown(&self, _scheduler: &mut Scheduler) {}
    fn cleanup(&self, _scheduler: &mut Scheduler) {}
}
