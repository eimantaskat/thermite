use std::fmt;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::handler::Response;

pub(crate) type Task = Pin<Box<dyn Future<Output = ()> + Send>>;

// clones share one list, so whoever holds one adds to what the response runs
#[derive(Clone, Default)]
pub struct BackgroundTasks(Arc<Mutex<Vec<Task>>>);

impl BackgroundTasks {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&self, task: impl Future<Output = ()> + Send + 'static) {
        self.tasks().push(Box::pin(task));
    }

    fn tasks(&self) -> MutexGuard<'_, Vec<Task>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl fmt::Debug for BackgroundTasks {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BackgroundTasks")
            .field("tasks", &self.tasks().len())
            .finish()
    }
}

// the server runs them once the response is sent
pub fn attach_background_tasks(response: &mut Response, tasks: BackgroundTasks) {
    response.extensions_mut().insert(tasks);
}

// what is added once the response is ready is not run
pub(crate) fn take_background_tasks(response: &mut Response) -> Vec<Task> {
    response
        .extensions_mut()
        .remove::<BackgroundTasks>()
        .map(|tasks| mem::take(&mut *tasks.tasks()))
        .unwrap_or_default()
}

// one after another, so a panic skips the tasks after it, as in Starlette
pub(crate) async fn run_in_order(tasks: Vec<Task>) {
    for task in tasks {
        task.await;
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;

    #[tokio::test]
    async fn runs_the_tasks_attached_to_a_response_in_the_order_they_were_added() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let record = |event| {
            let order = Arc::clone(&order);
            async move {
                order
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(event);
            }
        };
        let tasks = BackgroundTasks::new();
        let mut response = Response::new(Bytes::new());

        tasks.add(record("first"));
        attach_background_tasks(&mut response, tasks.clone());
        tasks.add(record("second"));
        run_in_order(take_background_tasks(&mut response)).await;

        assert_eq!(
            *order.lock().unwrap_or_else(PoisonError::into_inner),
            ["first", "second"]
        );
        assert!(take_background_tasks(&mut response).is_empty());
    }

    #[test]
    fn shows_how_many_tasks_it_holds() {
        let tasks = BackgroundTasks::new();
        tasks.add(async {});

        assert_eq!(format!("{tasks:?}"), "BackgroundTasks { tasks: 1 }");
    }
}
