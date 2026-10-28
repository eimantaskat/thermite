use std::any::{Any, TypeId, type_name};
use std::collections::HashMap;
use std::fmt;
use std::future::{Future, ready};
use std::sync::Arc;

use http::StatusCode;
use thermite_core::{Arguments, Request};

use crate::arity::for_each_arity;
use crate::extract::FromRequest;

#[derive(Debug)]
pub struct State<T>(pub Arc<T>);

// derive(Clone) would ask for T: Clone, while cloning only copies the Arc
impl<T> Clone for State<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Send + Sync + 'static> FromRequest for State<T> {
    type Rejection = StatusCode;

    fn require_state(required: &mut RequiredState) {
        required.add::<T>();
    }

    fn from_request(
        request: &Request,
        _: &Arguments,
    ) -> impl Future<Output = Result<Self, StatusCode>> + Send {
        let state = request
            .extensions()
            .get::<Arc<StateMap>>()
            .and_then(|states| Self::from_state(states).ok());
        ready(state.ok_or_else(|| {
            // a started app has checked every State<T> its routes take, so the app was never started
            tracing::error!(state = type_name::<T>(), "state is missing");
            StatusCode::INTERNAL_SERVER_ERROR
        }))
    }
}

// the state types something reads, so the app can check them all before it starts
#[derive(Debug, Default)]
pub struct RequiredState(Vec<StateType>);

#[derive(Debug, Clone, Copy)]
pub(crate) struct StateType {
    pub(crate) id: TypeId,
    pub(crate) name: &'static str,
}

impl RequiredState {
    fn add<T: 'static>(&mut self) {
        self.0.push(StateType {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
        });
    }

    pub(crate) fn first_missing(&self, available: impl Fn(TypeId) -> bool) -> Option<StateType> {
        self.0.iter().copied().find(|state| !available(state.id))
    }
}

#[derive(Clone, Default)]
pub struct StateMap(HashMap<TypeId, (&'static str, Arc<dyn Any + Send + Sync>)>);

impl StateMap {
    pub(crate) fn insert<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        self.0.insert(TypeId::of::<T>(), (type_name::<T>(), value));
    }

    pub(crate) fn contains(&self, id: TypeId) -> bool {
        self.0.contains_key(&id)
    }

    fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let (_, value) = self.0.get(&TypeId::of::<T>())?;
        Arc::clone(value).downcast().ok()
    }
}

impl fmt::Debug for StateMap {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_set()
            .entries(self.0.values().map(|(name, _)| name))
            .finish()
    }
}

// what a resource needs to start: State<T> or a tuple of them
pub trait FromState: Sized + Send + 'static {
    fn require_state(required: &mut RequiredState);

    // the error is the name of the first state type missing
    fn from_state(states: &StateMap) -> Result<Self, &'static str>;
}

impl<T: Send + Sync + 'static> FromState for State<T> {
    fn require_state(required: &mut RequiredState) {
        required.add::<T>();
    }

    fn from_state(states: &StateMap) -> Result<Self, &'static str> {
        states.get().map(Self).ok_or(type_name::<T>())
    }
}

macro_rules! impl_from_state_for_tuple {
    ($($state:ident),*) => {
        impl<$($state: FromState),*> FromState for ($($state,)*) {
            // the empty tuple needs nothing
            #[allow(unused_variables)]
            fn require_state(required: &mut RequiredState) {
                $($state::require_state(required);)*
            }

            // the empty tuple needs nothing, so it reads no state
            #[allow(unused_variables)]
            fn from_state(states: &StateMap) -> Result<Self, &'static str> {
                Ok(($($state::from_state(states)?,)*))
            }
        }
    };
}

for_each_arity!(impl_from_state_for_tuple);

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    struct Settings(&'static str);

    #[tokio::test]
    async fn answers_500_when_the_state_was_never_handed_to_the_request() {
        let rejection = State::<Settings>::from_request(&Request::default(), &Arguments::default())
            .await
            .err();

        assert_eq!(rejection, Some(StatusCode::INTERNAL_SERVER_ERROR));
    }

    #[tokio::test]
    async fn reads_the_state_of_its_type() {
        let mut states = StateMap::default();
        states.insert(Arc::new(Settings("thermite")));
        let mut request = Request::default();
        request.extensions_mut().insert(Arc::new(states));

        let state = State::<Settings>::from_request(&request, &Arguments::default())
            .await
            .unwrap();
        let State(settings) = state.clone();

        assert_eq!(settings.0, "thermite");
        assert!(Arc::ptr_eq(&settings, &state.0));
    }

    #[test]
    fn a_later_value_of_the_same_type_replaces_the_earlier_one() {
        let mut states = StateMap::default();
        states.insert(Arc::new(Settings("first")));
        states.insert(Arc::new(Settings("second")));

        let (State(settings),) = <(State<Settings>,)>::from_state(&states).unwrap();

        assert_eq!(settings.0, "second");
        assert_eq!(
            format!("{states:?}"),
            format!("{{{:?}}}", type_name::<Settings>())
        );
    }

    #[test]
    fn names_the_first_state_that_is_missing() {
        let mut states = StateMap::default();
        states.insert(Arc::new(Settings("thermite")));
        let mut required = RequiredState::default();
        <(State<Settings>, State<u8>, State<u16>) as FromState>::require_state(&mut required);

        let declared = required.first_missing(|id| states.contains(id));
        let read = <(State<Settings>, State<u8>, State<u16>)>::from_state(&states).err();

        assert_eq!(declared.map(|state| state.name), Some(type_name::<u8>()));
        assert_eq!(read, Some(type_name::<u8>()));
    }
}
