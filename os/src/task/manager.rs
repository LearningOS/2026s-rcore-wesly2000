//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::vec::Vec;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_vec: Vec<Arc<TaskControlBlock>>,
}

/// Scheduler using stride-based algorithm
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_vec: Vec::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_vec.push(task);
    }
    /// Take a process out of the ready vector
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if let Some((index, _)) = self.ready_vec.iter().
                                        enumerate().
                                        min_by_key(|(_, t)| t.inner_exclusive_access().stride()) {
            let task = self.ready_vec.swap_remove(index);
            task.inner_exclusive_access().increase_stride();
            return Some(task);
        }

        None
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
