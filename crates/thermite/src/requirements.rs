use std::any::{TypeId, type_name};

// what a route or resource needs, gathered once when it is added, so the app checks it before starting
#[derive(Debug, Default)]
pub struct Requirements {
    states: Vec<StateType>,
    request_scope: RequestScope,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct StateType {
    pub(crate) id: TypeId,
    pub(crate) name: &'static str,
}

// each costs allocations per request, so only the routes whose extractors use them get them
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RequestScope {
    pub(crate) dependency_cache: bool,
    pub(crate) background_tasks: bool,
}

impl Requirements {
    pub(crate) fn add_state<T: 'static>(&mut self) {
        self.states.push(StateType {
            id: TypeId::of::<T>(),
            name: type_name::<T>(),
        });
    }

    pub(crate) fn cache_dependencies(&mut self) {
        self.request_scope.dependency_cache = true;
    }

    pub(crate) fn collect_background_tasks(&mut self) {
        self.request_scope.background_tasks = true;
    }

    pub(crate) fn first_missing_state(
        &self,
        available: impl Fn(TypeId) -> bool,
    ) -> Option<StateType> {
        self.states
            .iter()
            .copied()
            .find(|state| !available(state.id))
    }

    pub(crate) fn request_scope(&self) -> RequestScope {
        self.request_scope
    }
}
