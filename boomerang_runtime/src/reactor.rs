use super::scheduler::SchedulerPoint;

slotmap::new_key_type! {
    pub struct ReactorKey;
}

pub trait ReactorElement<S: SchedulerPoint> {
    fn startup(&self, _sched: &S) {}
    fn shutdown(&self, _sched: &S) {}
    fn cleanup(&self, _sched: &S) {}
}
