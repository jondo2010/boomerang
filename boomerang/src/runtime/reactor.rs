use super::scheduler::Scheduler;

pub trait ReactorElement {
    fn startup(&self, _scheduler: &mut Scheduler) {}
    fn shutdown(&self, _scheduler: &mut Scheduler) {}
    fn cleanup(&self, _scheduler: &mut Scheduler) {}
}
