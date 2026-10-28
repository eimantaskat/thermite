use std::any::{TypeId, type_name};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use thermite_core::Request;
use tokio::sync::OnceCell;

use crate::resource::Resource;
use crate::state::{FromState, RequiredState, StateMap, StateType};

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartupError {
    #[error("{needed_by} needs the state {state}, but no .state() or .resource() adds it")]
    MissingState {
        state: &'static str,
        needed_by: String,
    },
    #[error("the resource {resource} is added more than once, as a resource or as state")]
    DuplicateResource { resource: &'static str },
    #[error("the resource {resource} needs itself")]
    NeedsItself { resource: &'static str },
    #[error("the resource {resource} needs {needs}, which is added after it")]
    ResourceOrder {
        resource: &'static str,
        needs: &'static str,
    },
    #[error("the resource {resource} failed to start")]
    Resource {
        resource: &'static str,
        source: Box<dyn Error + Send + Sync>,
    },
}

type BoxedFuture<'a, Output> = Pin<Box<dyn Future<Output = Output> + Send + 'a>>;

// the state and resources of an app, started before its first request and stopped after its last
#[derive(Default)]
pub(crate) struct Lifespan {
    states: StateMap,
    resources: Vec<Box<dyn Start>>,
    required: Vec<(String, RequiredState)>,
    running: OnceCell<Running>,
}

struct Running {
    states: Arc<StateMap>,
    resources: Vec<Arc<dyn Stop>>,
}

impl Lifespan {
    pub(crate) fn add_state<T: Send + Sync + 'static>(&mut self, value: T) {
        self.states.insert(Arc::new(value));
    }

    pub(crate) fn add_resource<T: Resource>(&mut self) {
        self.resources.push(Box::new(Starter::<T>(PhantomData)));
    }

    pub(crate) fn require(&mut self, needed_by: String, required: RequiredState) {
        self.required.push((needed_by, required));
    }

    pub(crate) async fn startup(&self) -> Result<(), StartupError> {
        self.running.get_or_try_init(|| self.start()).await?;
        Ok(())
    }

    pub(crate) async fn shutdown(&self) {
        if let Some(running) = self.running.get() {
            stop_in_reverse(&running.resources).await;
        }
    }

    pub(crate) fn hand_state_to(&self, request: &mut Request) {
        if let Some(running) = self.running.get() {
            request.extensions_mut().insert(Arc::clone(&running.states));
        }
    }

    async fn start(&self) -> Result<Running, StartupError> {
        self.check()?;
        let mut states = self.states.clone();
        let mut started = Vec::with_capacity(self.resources.len());
        for resource in &self.resources {
            match resource.start(&mut states).await {
                Ok(resource) => started.push(resource),
                Err(error) => {
                    stop_in_reverse(&started).await;
                    return Err(error);
                }
            }
        }
        Ok(Running {
            states: Arc::new(states),
            resources: started,
        })
    }

    // before any resource starts, so a mistake found here has no side effects to undo
    fn check(&self) -> Result<(), StartupError> {
        let added = |id| self.states.contains(id) || self.resource_starting(id).is_some();
        for (position, resource) in self.resources.iter().enumerate() {
            // a second start would replace the first value, while both get stopped
            if self.states.contains(resource.id())
                || self.resource_starting(resource.id()) != Some(position)
            {
                return Err(StartupError::DuplicateResource {
                    resource: resource.name(),
                });
            }
            let mut required = RequiredState::default();
            resource.require_state(&mut required);
            let started_before = |id| {
                self.states.contains(id)
                    || self
                        .resource_starting(id)
                        .is_some_and(|other| other < position)
            };
            if let Some(state) = required.first_missing(started_before) {
                return Err(if state.id == resource.id() {
                    StartupError::NeedsItself {
                        resource: resource.name(),
                    }
                } else if added(state.id) {
                    StartupError::ResourceOrder {
                        resource: resource.name(),
                        needs: state.name,
                    }
                } else {
                    missing(state, format!("the resource {}", resource.name()))
                });
            }
        }
        for (needed_by, required) in &self.required {
            if let Some(state) = required.first_missing(added) {
                return Err(missing(state, needed_by.clone()));
            }
        }
        Ok(())
    }

    // the position of the resource of this type, which is also its state
    fn resource_starting(&self, id: TypeId) -> Option<usize> {
        self.resources
            .iter()
            .position(|resource| resource.id() == id)
    }
}

impl fmt::Debug for Lifespan {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Lifespan")
            .field("states", &self.states)
            .field(
                "resources",
                &self
                    .resources
                    .iter()
                    .map(|resource| resource.name())
                    .collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

fn missing(state: StateType, needed_by: String) -> StartupError {
    StartupError::MissingState {
        state: state.name,
        needed_by,
    }
}

async fn stop_in_reverse(resources: &[Arc<dyn Stop>]) {
    for resource in resources.iter().rev() {
        resource.stop_boxed().await;
    }
}

// Resource is generic over its own type, so a list of them goes through these two traits
trait Start: Send + Sync {
    fn id(&self) -> TypeId;

    fn name(&self) -> &'static str;

    fn require_state(&self, required: &mut RequiredState);

    fn start<'a>(
        &self,
        states: &'a mut StateMap,
    ) -> BoxedFuture<'a, Result<Arc<dyn Stop>, StartupError>>;
}

trait Stop: Send + Sync {
    fn stop_boxed(&self) -> BoxedFuture<'_, ()>;
}

struct Starter<T>(PhantomData<fn() -> T>);

impl<T: Resource> Start for Starter<T> {
    fn id(&self) -> TypeId {
        TypeId::of::<T>()
    }

    fn name(&self) -> &'static str {
        type_name::<T>()
    }

    fn require_state(&self, required: &mut RequiredState) {
        T::Needs::require_state(required);
    }

    fn start<'a>(
        &self,
        states: &'a mut StateMap,
    ) -> BoxedFuture<'a, Result<Arc<dyn Stop>, StartupError>> {
        Box::pin(async move {
            let needs =
                T::Needs::from_state(states).map_err(|state| StartupError::MissingState {
                    state,
                    needed_by: format!("the resource {}", type_name::<T>()),
                })?;
            let resource = T::start(needs)
                .await
                .map_err(|source| StartupError::Resource {
                    resource: type_name::<T>(),
                    source: Box::new(source),
                })?;
            let resource = Arc::new(resource);
            states.insert(Arc::clone(&resource));
            Ok(resource as Arc<dyn Stop>)
        })
    }
}

impl<T: Resource> Stop for T {
    fn stop_boxed(&self) -> BoxedFuture<'_, ()> {
        Box::pin(self.stop())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::convert::Infallible;
    use std::future::ready;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::state::State;

    use super::*;

    struct Settings;

    static POOL_STARTS: AtomicUsize = AtomicUsize::new(0);

    struct Pool;

    impl Resource for Pool {
        type Needs = State<Settings>;
        type Error = Infallible;

        fn start(_: State<Settings>) -> impl Future<Output = Result<Self, Infallible>> + Send {
            POOL_STARTS.fetch_add(1, Ordering::SeqCst);
            ready(Ok(Self))
        }
    }

    fn lifespan() -> Lifespan {
        let mut lifespan = Lifespan::default();
        lifespan.add_state(Settings);
        lifespan.add_resource::<Pool>();
        lifespan
    }

    #[tokio::test]
    async fn starts_its_resources_once_however_often_startup_runs() {
        let lifespan = lifespan();

        lifespan.startup().await.unwrap();
        lifespan.startup().await.unwrap();
        lifespan.shutdown().await;

        assert_eq!(POOL_STARTS.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hands_no_state_to_requests_before_startup() {
        let lifespan = lifespan();
        let mut request = Request::default();

        lifespan.hand_state_to(&mut request);
        lifespan.shutdown().await;

        assert!(request.extensions().get::<Arc<StateMap>>().is_none());
    }

    #[tokio::test]
    async fn a_resource_started_without_its_needs_reports_them_missing() {
        let mut states = StateMap::default();

        let started = Starter::<Pool>(PhantomData).start(&mut states).await;

        assert!(matches!(
            started.err(),
            Some(StartupError::MissingState { state, .. }) if state == type_name::<Settings>()
        ));
    }

    #[test]
    fn debug_output_lists_the_states_and_resources() {
        assert_eq!(
            format!("{:?}", lifespan()),
            format!(
                "Lifespan {{ states: {{{:?}}}, resources: [{:?}], .. }}",
                type_name::<Settings>(),
                type_name::<Pool>()
            )
        );
    }
}
