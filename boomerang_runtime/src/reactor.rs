use super::scheduler::SchedulerPoint;

slotmap::new_key_type! {
    pub struct ReactorKey;
}

pub trait ReactorElement {
    fn startup(&self, _scheduler: &SchedulerPoint) {}
    fn shutdown(&self, _scheduler: &SchedulerPoint) {}
    fn cleanup(&self, _scheduler: &SchedulerPoint) {}
}
