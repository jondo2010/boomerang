use super::scheduler::SchedulerPoint;
use super::ActionKey;

slotmap::new_key_type! {
    pub struct ReactorKey;
}

pub trait ReactorElement<S: SchedulerPoint> {
    fn startup(&self, _sched: &S, _key: ActionKey) {}
    fn shutdown(&self, _sched: &S, _key: ActionKey) {}
    fn cleanup(&self, _sched: &S, _key: ActionKey) {}
}
