#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{
    cell::RefCell,
    sync::atomic::{AtomicBool, AtomicUsize},
    task::Waker,
};

use critical_section::Mutex;
use futures_util::task::AtomicWaker;
use intrusive::List;

mod group;
mod intrusive;
mod task;

pub use group::{ShutdownSignal, TaskGroup};
pub use task::Task;

pub struct State {
    running_tasks: AtomicUsize,
    done_waker: AtomicWaker,
    shutdown_wakers: Mutex<RefCell<List<Option<Waker>>>>,
    shutdown_signaled: AtomicBool,
}

impl State {
    pub const fn new() -> Self {
        State {
            running_tasks: AtomicUsize::new(0),
            done_waker: AtomicWaker::new(),
            shutdown_wakers: Mutex::new(RefCell::new(List::new())),
            shutdown_signaled: AtomicBool::new(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn spawns_tasks() {
        static STATE: State = State::new();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let group = TaskGroup::with_static(&STATE);
        let (tx, rx) = tokio::sync::oneshot::channel();

        runtime.block_on(async {
            let task = group.create(async move {
                if let Err(_) = tx.send(()) {
                    panic!("the receiver dropped");
                }
            });
            runtime.spawn(task);

            tokio::select! {
                result = rx => match result {
                    Ok(_) => {}
                    Err(_) => panic!("the sender did not spawn"),
                },
                _ = tokio::time::sleep(core::time::Duration::from_secs(10)) => panic!()
            }
        });
    }

    #[test]
    fn done_waits() {
        static STATE: State = State::new();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let group = TaskGroup::with_static(&STATE);

        runtime.block_on(async {
            let task = group.create(async move {
                loop {
                    tokio::time::sleep(core::time::Duration::from_millis(100)).await;
                }
            });
            runtime.spawn(task);

            tokio::select! {
                _ = group.done() => panic!(),
                _ = tokio::time::sleep(core::time::Duration::from_secs(2)) => {}
            }
        });
    }

    #[test]
    fn done_exits() {
        static STATE: State = State::new();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let group = TaskGroup::with_static(&STATE);

        runtime.block_on(async {
            for _ in 0..5 {
                let task = group.create(async move {
                    tokio::time::sleep(core::time::Duration::from_millis(100)).await;
                });
                runtime.spawn(task);
            }

            tokio::select! {
                _ = group.done() => {},
                _ = tokio::time::sleep(core::time::Duration::from_secs(2)) => panic!()
            }
        });
    }

    #[test]
    fn shutdown_signals() {
        static STATE: State = State::new();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let group = TaskGroup::with_static(&STATE);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();

        runtime.block_on(async {
            for _ in 0..5 {
                let tx = tx.clone();
                let task = group.create_with_shutdown(|shutdown| async move {
                    tokio::select! {
                        _ = shutdown => {},
                        _ = tokio::time::sleep(core::time::Duration::from_secs(5)) => {
                            let _ = tx.send(());
                        },
                    }
                    core::mem::drop(tx);
                });
                runtime.spawn(task);
            }

            core::mem::drop(tx);
            tokio::time::sleep(core::time::Duration::from_secs(1)).await;
            group.shutdown().await;
            if let Some(_) = rx.recv().await {
                panic!();
            }
        });
    }
}
