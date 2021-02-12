use super::scheduler::SchedulerPoint;

slotmap::new_key_type! {
    pub struct ReactorKey;
}

pub trait ReactorElement {
    fn startup(&self, _sched: &SchedulerPoint) {}
    fn shutdown(&self, _sched: &SchedulerPoint) {}
    fn cleanup(&self, _sched: &SchedulerPoint) {}
}
